package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func nativeScenario(id, level string) bool {
	switch id {
	case "HELPER-RESTART-REAL", "HELPER-TCC-READY", "REM-CREATE-EXTERNAL", "REM-LIST-CREATE", "REM-RECURRING", "REM-PERMISSION-LOSS":
		return true
	case "REM-CRUD":
		return level == "real" || level == "replica"
	case "CORE-LIVE-LATENCY":
		return level == "real"
	default:
		return false
	}
}

func runNativeScenario(ctx context.Context, s Scenario, level string) (result Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level == "replica" {
		e.Hosts = append(e.Hosts, "ubuntu")
	}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	blocked := func(reason string) Evidence {
		e.Status = "unverified"
		e.Assertions = append(e.Assertions, Assertion{Name: "independent_truth", Passed: false, Detail: reason})
		return e
	}
	failed := func(name string) Evidence { add(name, false); return e }
	var w *world.World
	var err error
	if level == "sim" {
		w, err = world.New(ctx, level, 1, 0)
	} else {
		w, err = world.NewReal(ctx, level, 1)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "native world startup:", err)
		return failed("world_start")
	}
	defer w.Close()
	ready := func() bool {
		records, err := w.Records(w.Gateway)
		if err != nil {
			return false
		}
		for _, record := range records {
			var data map[string]any
			json.Unmarshal(record.Data, &data)
			if record.Collection == "system.source" && data["source"] == "reminders" && data["state"] == "ready" {
				return true
			}
		}
		return false
	}
	if w.Await(15*time.Second, ready) != nil {
		return failed("reminders_ready")
	}
	add("reminders_ready", true)
	checkpointPath := filepath.Join(w.Gateway.Home, "helper", "lab-committed-reminders")
	if level != "sim" && w.Await(5*time.Second, func() bool { _, err := os.Stat(checkpointPath); return err == nil }) != nil {
		return failed("native_capture_committed")
	}
	restart := func() error {
		before, _ := os.ReadFile(checkpointPath)
		if err := w.RestartHelper(); err != nil {
			return err
		}
		return w.Await(5*time.Second, func() bool {
			current, err := os.ReadFile(checkpointPath)
			return err == nil && string(current) != string(before) && ready()
		})
	}
	if s.ID == "HELPER-TCC-READY" {
		app := filepath.Join(os.Getenv("HOME"), "Applications/AppleBridgeDev/AppleBridgeHelper.app")
		permissions := func() (map[string]string, error) {
			path := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev", filepath.Base(w.Dir)+"-permissions.json")
			defer os.Remove(path)
			if err := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "status", "--report", path).Run(); err != nil {
				return nil, err
			}
			data, err := os.ReadFile(path)
			var values map[string]string
			if err == nil {
				err = json.Unmarshal(data, &values)
			}
			return values, err
		}
		before, err := permissions()
		if err != nil {
			return failed("permission_report")
		}
		requirementBefore, err := exec.CommandContext(ctx, "codesign", "-d", "-r-", app).CombinedOutput()
		if err != nil {
			return failed("designated_requirement")
		}
		command := exec.CommandContext(ctx, "packaging/dev/build-helper.sh")
		if output, err := command.CombinedOutput(); err != nil {
			fmt.Fprintln(os.Stderr, string(output))
			return failed("signed_rebuild")
		}
		after, err := permissions()
		if err != nil {
			return failed("permission_report_after_rebuild")
		}
		requirementAfter, err := exec.CommandContext(ctx, "codesign", "-d", "-r-", app).CombinedOutput()
		add("designated_requirement_preserved", err == nil && string(requirementBefore) == string(requirementAfter))
		add("permission_states_preserved", reflect.DeepEqual(before, after))
		add("granted_access_preserved", after["full_disk_access"] == "granted" && after["reminders"] == "granted" && after["accessibility"] == "granted" && after["automation_messages"] == "granted")
		add("calendar_contacts_states_preserved", before["calendar"] == after["calendar"] && before["contacts"] == after["contacts"])
		add("real_capture_after_rebuild", restart() == nil)
	} else {
		if s.ID == "HELPER-RESTART-REAL" {
			before, err := w.Records(w.Gateway)
			if err != nil {
				return failed("before_restart")
			}
			if restart() != nil {
				return failed("helper_restarted")
			}
			after, err := w.Records(w.Gateway)
			add("restart_preserves_records", err == nil && reflect.DeepEqual(recordMultiset(before, false), recordMultiset(after, false)))
		}
		tag := "[ab-test " + contracts.NewID("m1") + "]"
		title := tag + " reminder"
		if level != "sim" {
			if _, err := w.Truth("observe", map[string]any{"source": "reminders", "list": "Bridge Test", "tag": tag}); err != nil {
				return blocked("Grant AppleTruth.app Reminders full access; no fixture changes were made")
			}
			defer func() {
				if _, err := w.Truth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": tag}); err != nil {
					result.Passed = false
					result.Assertions = append(result.Assertions, Assertion{Name: "tagged_fixture_cleanup", Passed: false})
				}
			}()
		}
		fixture := func(action string, options map[string]any) error {
			options["action"] = action
			options["list"] = "Bridge Test"
			options["tag"] = tag
			if level != "sim" {
				_, err := w.Truth("fixture", options)
				return err
			}
			if action == "create_list" {
				return nil
			}
			if action == "create" {
				data := map[string]any{"title": options["title"], "completed": false, "list_id": map[string]any{"$ref": map[string]string{"collection": "reminders.list", "native_key": "list"}}}
				if options["recurring"] == true {
					data["recurrence"] = "daily"
					data["coverage"] = "next_incomplete_instance_only"
				}
				return w.ControlAction("/external-change", map[string]any{"key": options["title"], "data": data})
			}
			return fmt.Errorf("unsupported fixture action")
		}
		find := func(endpoint world.Endpoint, collection, title string) (contracts.Record, bool) {
			records, err := w.Records(endpoint)
			if err != nil {
				return contracts.Record{}, false
			}
			for _, record := range records {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				if record.Collection == collection && data["title"] == title {
					return record, true
				}
			}
			return contracts.Record{}, false
		}
		var lastCommand map[string]any
		submit := func(operation string, args map[string]any, version uint64) error {
			request := map[string]any{"command_id": contracts.NewID("cmd"), "args": args, "expected_version": version, "wait_ms": 3000}
			var result map[string]any
			endpoint := w.Gateway
			if level == "replica" {
				endpoint = w.Replicas[0]
			}
			_, err := world.Request(ctx, endpoint, "/v1/op/"+operation, request, &result)
			if err != nil {
				return err
			}
			lastCommand = result
			if result["disposition"] != "locally_observed" {
				return fmt.Errorf("command did not produce observed evidence")
			}
			return nil
		}
		switch s.ID {
		case "CORE-LIVE-LATENCY":
			endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
			observers := []*watcher{}
			for _, endpoint := range endpoints {
				observer, err := watch(ctx, endpoint, false, func(contracts.Record) bool { return false })
				if err != nil {
					return failed("latency_observer")
				}
				defer observer.close()
				observers = append(observers, observer)
			}
			burst, err := w.Truth("fixture", map[string]any{"action": "burst", "list": "Bridge Test", "tag": tag, "count": 200})
			if err != nil {
				return blocked("AppleTruth burst could not be verified")
			}
			effects, ok := burst["effects"].([]any)
			if !ok || len(effects) != 200 {
				return failed("two_hundred_external_effects")
			}
			complete := w.Await(10*time.Second, func() bool {
				for _, raw := range effects {
					effect, ok := raw.(map[string]any)
					if !ok {
						return false
					}
					title, _ := effect["title"].(string)
					for _, observer := range observers {
						if _, ok := observer.observedExact(title); !ok {
							return false
						}
					}
				}
				return true
			})
			add("all_external_effects_visible", complete == nil)
			for index, observer := range observers {
				values := []float64{}
				timeouts := 0
				for _, raw := range effects {
					effect := raw.(map[string]any)
					title, _ := effect["title"].(string)
					rawTime, ok := effect["effect_uptime_ns"].(json.Number)
					if !ok {
						return failed("native_monotonic_timestamp")
					}
					effectTime, err := rawTime.Int64()
					seen, visible := observer.observedExact(title)
					if err != nil || effectTime <= 0 || (visible && seen.UptimeNS < effectTime) {
						return failed("monotonic_clock_agreement")
					}
					if !visible {
						timeouts++
						continue
					}
					values = append(values, float64(seen.UptimeNS-effectTime)/float64(time.Millisecond))
				}
				name := "effect_done→gateway_visible"
				if index > 0 {
					name = fmt.Sprintf("effect_done→replica_visible:replica%d", index-1)
				}
				latency := stats(values, timeouts)
				e.Latency[name] = latency
				add("latency_"+name, latency.Samples == 200 && latency.Timeouts == 0 && latency.P95 < 1000)
				add("contiguous_replica_application", observer.valid())
			}
		case "REM-LIST-CREATE":
			if err := submit("reminders.create_list", map[string]any{"title": "Bridge Test"}, 0); err != nil {
				return failed("rest_create_list")
			}
			if level != "sim" {
				evidence, ok := lastCommand["evidence"].(map[string]any)
				if !ok {
					return failed("created_list_evidence")
				}
				nativeKey, _ := evidence["native_key"].(string)
				if nativeKey == "" {
					return failed("created_list_identity")
				}
				defer func() {
					if _, err := w.Truth("fixture", map[string]any{"action": "cleanup_list", "list": "Bridge Test", "list_id": nativeKey, "tag": tag}); err != nil {
						result.Passed = false
						result.Assertions = append(result.Assertions, Assertion{Name: "created_list_cleanup", Passed: false})
					}
				}()
				truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "list_id": nativeKey, "tag": tag})
				add("truth_created_list", err == nil && truth["list_exists"] == true && truth["list_id"] == nativeKey)
			}
			add("list_visible", w.Await(3*time.Second, func() bool { _, ok := find(w.Gateway, "reminders.list", "Bridge Test"); return ok }) == nil)
		case "REM-PERMISSION-LOSS":
			if fixture("create", map[string]any{"title": title}) != nil {
				return failed("fixture_create")
			}
			if w.ControlAction("/flush", struct{}{}) != nil {
				return failed("capture_committed")
			}
			before, _ := w.Records(w.Gateway)
			if w.ControlAction("/source-state", map[string]string{"source": "reminders", "state": "permission_missing"}) != nil {
				return failed("permission_loss_fault")
			}
			add("permission_missing_reported", w.Await(3*time.Second, func() bool { return !ready() }) == nil)
			if w.ControlAction("/external-change", map[string]any{"key": title, "delete": true}) != nil {
				return failed("external_delete_while_denied")
			}
			if sleep(ctx, 150*time.Millisecond) != nil {
				return failed("permission_settle")
			}
			after, err := w.Records(w.Gateway)
			add("no_false_deletion", err == nil && reflect.DeepEqual(recordMultiset(before, false), recordMultiset(after, false)))
		default:
			if s.ID == "REM-CRUD" {
				if fixture("create_list", map[string]any{}) != nil {
					return failed("fixture_list")
				}
				var list contracts.Record
				if w.Await(3*time.Second, func() bool { var ok bool; list, ok = find(w.Gateway, "reminders.list", "Bridge Test"); return ok }) != nil {
					return failed("list_visible")
				}
				if submit("reminders.create", map[string]any{"list_id": list.ID, "title": title}, 0) != nil {
					return failed("rest_reminders.create")
				}
				add("rest_reminders.create", true)
			} else if fixture("create", map[string]any{"title": title, "recurring": s.ID == "REM-RECURRING"}) != nil {
				return failed("fixture_create")
			}
			effectDone := time.Now()
			values := []float64{}
			if w.Await(3*time.Second, func() bool {
				for _, endpoint := range w.Replicas {
					if _, ok := find(endpoint, "reminders.reminder", title); !ok {
						return false
					}
				}
				return true
			}) != nil {
				return failed("replica_visible")
			}
			values = append(values, float64(time.Since(effectDone))/float64(time.Millisecond))
			e.Latency["effect_done→replica_visible:replica0"] = stats(values, 0)
			add("replica_visible_under_one_second", values[0] < 1000)
			if s.ID == "REM-CRUD" {
				for _, operation := range []string{"reminders.update", "reminders.complete", "reminders.uncomplete", "reminders.delete"} {
					var record contracts.Record
					if w.Await(3*time.Second, func() bool { var ok bool; record, ok = find(w.Gateway, "reminders.reminder", title); return ok }) != nil {
						return failed("command_target_visible")
					}
					args := map[string]any{"id": record.ID}
					if operation == "reminders.update" {
						title = tag + " updated"
						args["title"] = title
					}
					if submit(operation, args, record.Version) != nil {
						return failed("rest_" + operation)
					}
					add("rest_"+operation, true)
					completed := operation == "reminders.complete"
					visible := w.Await(3*time.Second, func() bool {
						for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
							current, exists := find(endpoint, "reminders.reminder", title)
							if operation == "reminders.delete" {
								if exists {
									return false
								}
								continue
							}
							var fields map[string]any
							json.Unmarshal(current.Data, &fields)
							if !exists || fields["completed"] != completed {
								return false
							}
						}
						return true
					})
					if visible != nil {
						return failed("read_your_write_" + operation)
					}
					if level != "sim" {
						truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": title})
						if err != nil {
							return blocked("AppleTruth access lost during CRUD")
						}
						records, _ := truth["records"].([]any)
						if operation == "reminders.delete" {
							if len(records) != 0 {
								return failed("truth_absence")
							}
						} else {
							if len(records) != 1 {
								return failed("truth_unique_reminder")
							}
							data, ok := records[0].(map[string]any)
							if !ok || data["completed"] != completed || data["title"] != title {
								return failed("truth_" + operation)
							}
						}
					}
				}
			}
			if s.ID == "REM-RECURRING" {
				original, ok := find(w.Gateway, "reminders.reminder", title)
				if !ok {
					return failed("recurring_original_visible")
				}
				if level == "sim" {
					if w.ControlAction("/complete-recurring", map[string]string{"key": title}) != nil {
						return failed("complete_recurring_fixture")
					}
				} else if fixture("complete", map[string]any{"title": title}) != nil {
					return failed("complete_recurring_fixture")
				}
				advanced := w.Await(5*time.Second, func() bool {
					records, err := w.Records(w.Gateway)
					if err != nil {
						return false
					}
					for _, record := range records {
						var data map[string]any
						json.Unmarshal(record.Data, &data)
						var previous map[string]any
						json.Unmarshal(original.Data, &previous)
						if record.Collection == "reminders.reminder" && data["title"] == title && data["completed"] == false && (record.ID != original.ID || data["due"] != previous["due"]) {
							return true
						}
					}
					return false
				})
				add("next_incomplete_recurring_instance", advanced == nil)
				if level != "sim" {
					truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag})
					records, _ := truth["records"].([]any)
					incomplete := 0
					for _, raw := range records {
						if data, ok := raw.(map[string]any); ok && data["completed"] == false && data["recurring"] == true {
							incomplete++
						}
					}
					add("truth_next_incomplete_instance", err == nil && incomplete == 1)
				}
			}
			if s.ID == "HELPER-RESTART-REAL" {
				helperHome := filepath.Join(w.Gateway.Home, "helper")
				arm := func(point string) error {
					return os.WriteFile(filepath.Join(helperHome, "lab-pause-"+point), []byte("armed"), 0600)
				}
				paused := func(point string) bool {
					_, err := os.Stat(filepath.Join(helperHome, "lab-paused-"+point))
					return err == nil
				}
				clear := func(point string) {
					os.Remove(filepath.Join(helperHome, "lab-pause-"+point))
					os.Remove(filepath.Join(helperHome, "lab-paused-"+point))
				}
				defer clear("capture")
				defer clear("dispatch")
				replayTitle := tag + " replay"
				if os.WriteFile(filepath.Join(helperHome, "lab-pause-capture"), []byte(replayTitle), 0600) != nil {
					return failed("arm_capture_pause")
				}
				if fixture("create", map[string]any{"title": replayTitle}) != nil {
					return failed("replay_fixture")
				}
				if w.Await(3*time.Second, func() bool { return paused("capture") }) != nil {
					return failed("capture_committed_before_helper_ack")
				}
				before, exists := find(w.Gateway, "reminders.reminder", replayTitle)
				if !exists {
					return failed("capture_durable_before_ack")
				}
				checkpointPath := filepath.Join(helperHome, "lab-committed-reminders")
				checkpointBefore, _ := os.ReadFile(checkpointPath)
				if w.RestartHelper() != nil {
					return failed("restart_mid_capture")
				}
				clear("capture")
				resumed := w.Await(5*time.Second, func() bool {
					current, ok := find(w.Gateway, "reminders.reminder", replayTitle)
					checkpoint, err := os.ReadFile(checkpointPath)
					return ok && current.ID == before.ID && current.Version == before.Version && err == nil && string(checkpoint) != string(checkpointBefore)
				})
				add("capture_replay_without_duplicate_version", resumed == nil)
				var list contracts.Record
				if w.Await(3*time.Second, func() bool { var ok bool; list, ok = find(w.Gateway, "reminders.list", "Bridge Test"); return ok }) != nil {
					return failed("command_list")
				}
				if arm("dispatch") != nil {
					return failed("arm_dispatch_pause")
				}
				commandID := contracts.NewID("cmd")
				commandTitle := tag + " guarded command"
				request := map[string]any{"command_id": commandID, "args": map[string]any{"list_id": list.ID, "title": commandTitle}, "wait_ms": 3000}
				returned := make(chan struct{})
				go func() {
					defer close(returned)
					var output map[string]any
					world.Request(ctx, w.Gateway, "/v1/op/reminders.create", request, &output)
				}()
				if w.Await(3*time.Second, func() bool { return paused("dispatch") }) != nil {
					return failed("durable_dispatch_guard_before_effect")
				}
				if w.RestartHelper() != nil {
					return failed("restart_mid_command")
				}
				clear("dispatch")
				select {
				case <-returned:
				case <-ctx.Done():
					return failed("command_returned_after_disconnect")
				}
				indeterminate := w.Await(5*time.Second, func() bool {
					var status map[string]any
					_, err := world.Request(ctx, w.Gateway, "/v1/op/commands.status", map[string]any{"command_id": commandID}, &status)
					return err == nil && status["state"] == "indeterminate" && status["disposition"] == "indeterminate"
				})
				add("unfinished_dispatch_indeterminate", indeterminate == nil)
				if sleep(ctx, 500*time.Millisecond) != nil {
					return failed("post_restart_settle")
				}
				truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": commandTitle})
				records, _ := truth["records"].([]any)
				add("no_automatic_redispatch", err == nil && len(records) == 0)
			}
		}
		if level == "sim" && s.ID != "REM-PERMISSION-LOSS" {
			add("independent_simtruth_convergence", w.Await(5*time.Second, func() bool { return nativeConvergence(w) }) == nil)
		} else if level != "sim" {
			result, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag})
			add("independent_appletruth", err == nil && result["status"] == "verified")
		}
	}
	gateway, err := w.Records(w.Gateway)
	if err != nil {
		return failed("gateway_records")
	}
	add("replica_convergence", w.Await(5*time.Second, func() bool {
		for _, endpoint := range w.Replicas {
			records, err := w.Records(endpoint)
			if err != nil || !reflect.DeepEqual(gateway, records) {
				return false
			}
		}
		return true
	}) == nil)
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		token, err := w.ScopedToken(endpoint, "system:read")
		if err != nil {
			return failed("scope_token")
		}
		endpoint.Token = token
		var result contracts.ReadResult
		_, err = world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &result)
		add("no_scope_leak", err == nil && len(result.Records) == 0)
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		if !assertion.Passed {
			e.Passed = false
		}
	}
	return e
}

func (w *watcher) observedExact(title string) (Seen, bool) {
	w.mu.Lock()
	defer w.mu.Unlock()
	for _, seen := range w.seen {
		if seen.Change.Record.Collection == "reminders.reminder" && recordTitle(seen.Change.Record) == title {
			return seen, true
		}
	}
	return Seen{}, false
}

func nativeConvergence(w *world.World) bool {
	truth, err := w.Inspect()
	if err != nil {
		return false
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return false
	}
	expected, err := truthRecords(truth, aliases)
	if err != nil {
		return false
	}
	gateway, err := w.Records(w.Gateway)
	if err != nil {
		return false
	}
	domain := []contracts.Record{}
	for _, record := range gateway {
		if !strings.HasPrefix(record.Collection, "system.") {
			domain = append(domain, record)
		}
	}
	if !reflect.DeepEqual(recordMultiset(domain, false), recordMultiset(expected, false)) {
		return false
	}
	for _, endpoint := range w.Replicas {
		replica, err := w.Records(endpoint)
		if err != nil || !reflect.DeepEqual(gateway, replica) {
			return false
		}
	}
	return true
}
