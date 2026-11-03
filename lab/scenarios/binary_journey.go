package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"reflect"
	"strings"
	"time"
)

func runBinaryScenario(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	evidence := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level == "replica" {
		evidence.Hosts = append(evidence.Hosts, "ubuntu")
	}
	add := func(name string, ok bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: ok})
	}
	fail := func(name string) Evidence { add(name, false); return evidence }
	if len(s.Then) == 1 {
		if options, ok := s.Then[0]["load_rejected"]; ok {
			path := options["path"].(string)
			_, err := Load([]string{path})
			add("malformed_scenario_rejected", err != nil && strings.Contains(err.Error(), "p95_ms"))
			evidence.Passed = evidence.Assertions[0].Passed
			if err != nil {
				evidence.Assertions[0].Detail = err.Error()
			}
			return evidence
		}
	}
	replicas, seed := 1, 4
	for _, step := range s.Given {
		for action, options := range step {
			switch action {
			case "world":
				replicas = number(options, "replicas", 1)
				if options["gateway_dials"] == true {
					ctx = world.WithGatewayDials(ctx)
				}
			case "seed":
				seed = number(options, "count", 4)
			default:
				return fail("known_given_action")
			}
		}
	}
	for _, step := range s.When {
		if options, ok := step["robustness"]; ok {
			return runRobust(ctx, s, level, options, evidence)
		}
	}
	options := []world.BlobOptions{}
	if strings.HasPrefix(s.ID, "CORE-BLOB-") {
		opt := world.BlobOptions{GraceMS: 500}
		if s.ID == "CORE-BLOB-CAP" {
			opt.Cap = 1 << 20
		}
		opt.Corrupt = s.ID == "CORE-BLOB-CORRUPT"
		opt.Slow = s.ID == "CORE-BLOB-INTERRUPT"
		options = append(options, opt)
	}
	w, err := world.New(ctx, level, replicas, seed, options...)
	if err != nil {
		fmt.Fprintln(os.Stderr, "world startup:", err)
		return fail("world_start")
	}
	defer w.Close()
	evidence.NetworkPath = w.NetworkPath
	add("feed_network_"+w.NetworkPath, true)
	add("replica_host_is_"+map[bool]string{true: "ubuntu", false: "air"}[level == "replica"], true)
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		return fail("initial_convergence")
	}

	for _, e := range w.Replicas {
		status, err := w.Status(e)
		if err != nil || status.Snapshots != 1 {
			return fail("bootstrap_snapshot")
		}
	}
	add("bootstrap_snapshot", true)
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	watchers := []*watcher{}
	for _, e := range endpoints {
		observer, err := watch(ctx, e, false, func(r contracts.Record) bool { return truthAbsent(w, r) })
		if err != nil {
			return fail("sse_observe")
		}
		defer observer.close()
		watchers = append(watchers, observer)
	}
	scoped := []*watcher{}
	for _, e := range endpoints {
		token, err := w.ScopedToken(e, "system:read")
		if err != nil {
			return fail("scope_token")
		}
		e.Token = token
		var result contracts.ReadResult
		_, err = world.Request(ctx, e, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder", IncludeAliases: true}, &result)
		if err != nil || len(result.Records) != 0 || len(result.Aliases) != 0 {
			return fail("scope_query")
		}
		all, err := w.Records(endpoints[0])
		if err != nil {
			return fail("scope_fixture")
		}
		for _, r := range all {
			if r.Collection == "reminders.reminder" {
				status, _ := world.Request(ctx, e, "/v1/records/"+r.Collection+"/"+r.ID, nil, nil)
				if status != 404 {
					return fail("scope_get")
				}
				break
			}
		}
		observer, err := watch(ctx, e, true, func(r contracts.Record) bool { return truthAbsent(w, r) })
		if err != nil {
			return fail("scope_sse")
		}
		defer observer.close()
		scoped = append(scoped, observer)
	}
	add("scope_query_and_get", true)
	for _, step := range s.When {
		if _, ok := step["command_journey"]; ok {
			if w.ControlAction("/external-change", map[string]string{"key": "command-probe"}) != nil {
				return fail("command_probe")
			}
			effectStart := time.Now()
			if w.Await(5*time.Second, func() bool {
				for _, observer := range watchers {
					if _, ok := observer.observed("command-probe"); !ok {
						return false
					}
				}
				return true
			}) != nil {
				return fail("healthy_command_probe_visible")
			}
			probeSeen := make([]Seen, len(watchers))
			for i, observer := range watchers {
				probeSeen[i], _ = observer.observed("command-probe")
			}
			runCommands(ctx, w, s.ID, &evidence, watchers)
			if s.ID == "CORE-CMD-ERA" {
				head, _ := w.Cursor(w.Gateway)
				all := append(append([]*watcher{}, watchers...), scoped...)
				reset := w.Await(5*time.Second, func() bool {
					for _, observer := range all {
						observer.mu.Lock()
						epoch := observer.active.Epoch
						observer.mu.Unlock()
						if epoch != head.Epoch {
							return false
						}
					}
					return true
				})
				add("all_sse_observers_resume_new_epoch", reset == nil)
				add("post_restore_probe", w.ControlAction("/external-change", map[string]string{"key": "post-restore-probe"}) == nil)
				add("post_restore_probe_committed", w.ControlAction("/flush", struct{}{}) == nil)
				head, _ = w.Cursor(w.Gateway)
				advanced := w.Await(5*time.Second, func() bool {
					for _, observer := range all {
						observer.mu.Lock()
						last := observer.last
						observer.mu.Unlock()
						if last.Epoch != head.Epoch || last.Sequence < head.Sequence {
							return false
						}
					}
					return true
				})
				add("all_sse_observers_advance_after_restore", advanced == nil)
			}
			for i, observer := range watchers {
				seen := probeSeen[i]
				ok := !seen.At.IsZero()
				values := []float64{}
				timeouts := 0
				if ok {
					values = append(values, max(0, float64(seen.At.Sub(effectStart))/float64(time.Millisecond)))
				} else {
					timeouts = 1
				}
				name := "effect_done→gateway_visible"
				if i > 0 {
					name = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
				}
				evidence.Latency[name] = stats(values, timeouts)
				add(fmt.Sprintf("metadata_latency_below_1s_%d", i), ok && seen.At.Sub(effectStart) < time.Second)
				add(fmt.Sprintf("sse_contiguous_watermarks_%d", i), observer.valid())
				observer.mu.Lock()
				noFalseDelete := !observer.falseDelete
				observer.mu.Unlock()
				add(fmt.Sprintf("no_false_deletion_%d", i), noFalseDelete)
			}
			for i, observer := range scoped {
				observer.mu.Lock()
				advanced := observer.last.Sequence > observer.initialSequence
				observer.mu.Unlock()
				add(fmt.Sprintf("scope_sse_watermarks_%d", i), observer.valid() && advanced)
			}
			recordIntegrity(&evidence, w, nil)
			evidence.Passed = len(evidence.Assertions) > 0
			for _, a := range evidence.Assertions {
				evidence.Passed = evidence.Passed && (a.Passed || a.Status == "unverified")
			}
			return evidence
		}
	}
	effects := map[string]time.Time{}
	partitionEffects := map[string]time.Time{}
	change := func(title string, measure bool) error {
		if err := w.ControlAction("/external-change", map[string]string{"key": title}); err != nil {
			return err
		}
		if measure {
			effects[title] = time.Now()
		} else {
			partitionEffects[title] = time.Now()
		}
		return nil
	}
	if err = change("probe", true); err != nil {
		return fail("external_change")
	}
	for _, step := range s.When {
		for action, options := range step {
			switch action {
			case "truth_multiplicity":
				data := json.RawMessage(`{"title":"[ab-test] duplicate","completed":false}`)
				for _, key := range []string{"duplicate-a", "duplicate-b"} {
					if err = w.ControlAction("/external-change", map[string]any{"key": key, "data": data, "omit_capture": key == "duplicate-b"}); err != nil {
						return fail("multiplicity_fixture")
					}
				}
				if err = w.ControlAction("/flush", struct{}{}); err != nil {
					return fail("multiplicity_flush")
				}
				truth, truthErr := w.Inspect()
				records, recordsErr := w.Records(w.Gateway)
				truthCount, gatewayCount := 0, 0
				for _, o := range truth {
					if recordTitle(contracts.Record{Data: o.Data}) == "[ab-test] duplicate" {
						truthCount++
					}
				}
				for _, record := range records {
					if recordTitle(record) == "[ab-test] duplicate" {
						gatewayCount++
					}
				}
				add("multiplicity_fault_two_truth_one_gateway", truthErr == nil && recordsErr == nil && truthCount == 2 && gatewayCount == 1)
				detected := truthErr == nil && recordsErr == nil && truthCount == 2 && gatewayCount == 1 && !convergence(w)
				add("truth_multiplicity_mismatch_detected", detected)
				evidence.Passed = detected
				return evidence
			case "blobs":
				if !blobJourney(ctx, w, s.ID, number(options, "rows", 10000), &evidence, add) {
					return evidence
				}
				if s.ID == "CORE-BLOB-REFERENCE-INDEX" {
					evidence.Passed = true
					for _, assertion := range evidence.Assertions {
						evidence.Passed = evidence.Passed && assertion.Passed
					}
					return evidence
				}
			case "gateway_commands":
				gatewayCommands(ctx, w, &evidence)
			case "connect":
			case "changes":
				delete(effects, "probe")
				count, rate := number(options, "count", 200), number(options, "per_second", 5)
				if count < 1 || rate < 1 {
					return fail("change_parameters")
				}
				next := time.Now()
				var finishCalendar func() bool
				for i := 0; i < count; i++ {
					if s.ID == "CORE-LIVE-LATENCY" && i == count/2 {
						finishCalendar = startCalendarLiveDelta(ctx, w, &evidence, watchers)
					}
					if err = change(fmt.Sprintf("live-%03d", i), true); err != nil {
						return fail("external_change")
					}
					next = next.Add(time.Second / time.Duration(rate))
					if delay := time.Until(next); delay > 0 {
						if err = sleep(ctx, delay); err != nil {
							return fail("canceled")
						}
					}
				}
				if finishCalendar != nil {
					add("calendar_200kb_delta_atomic_during_reminders_live", finishCalendar())
				}
			case "disconnect":
				if w.Await(5*time.Second, func() bool { return convergence(w) }) != nil {
					return fail("pre_disconnect_convergence")
				}
				before, err := w.Records(w.Replicas[0])
				if err != nil {
					return fail("reads_before_disconnect")
				}
				w.Blackhole()
				if err = w.Await(15*time.Second, func() bool {
					status, err := w.Status(w.Replicas[0])
					return err == nil && status.State == "disconnected"
				}); err != nil {
					return fail("disconnected_within_15_seconds")
				}
				after, err := w.Records(w.Replicas[0])
				if err != nil || !reflect.DeepEqual(before, after) {
					return fail("local_reads_survive_disconnect")
				}
				add("silent_blackhole_detected_within_15_seconds_reads_continue", true)
				if err = w.Heal(0); err != nil {
					return fail("heal")
				}
				if err = change("reconnected", true); err != nil {
					return fail("reconnect_change")
				}
			case "partition":
				if err = w.Await(5*time.Second, func() bool { _, ok := watchers[1].observed("probe"); return ok }); err != nil {
					return fail("pre_partition_visible")
				}
				w.Partition(0)
				seconds, count := number(options, "seconds", 30), number(options, "changes", 30)
				for i := 0; i < count; i++ {
					if err = change(fmt.Sprintf("offline-%03d", i), false); err != nil {
						return fail("partition_change")
					}
					if err = sleep(ctx, time.Duration(seconds)*time.Second/time.Duration(count)); err != nil {
						return fail("canceled")
					}
				}
			case "heal":
				if err = w.Heal(0); err != nil {
					return fail("heal")
				}
				if err = w.Await(15*time.Second, func() bool { status, err := w.Status(w.Replicas[0]); return err == nil && status.State == "connected" }); err != nil {
					return fail("reconnect")
				}
				if err = change("healed", true); err != nil {
					return fail("heal_change")
				}
			default:
				return fail("known_when_action")
			}
		}
	}
	if s.ID == "CORE-GATEWAY-DIALS" {
		delete(effects, "healed")
	}
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		fmt.Fprintln(os.Stderr, "convergence diagnosis:", convergenceFailure(w))
		return fail("convergence")
	}
	add("convergence_truth_gateway_replicas", true)
	timeout := w.Await(10*time.Second, func() bool {
		for title := range effects {
			for _, observer := range watchers {
				if _, ok := observer.observed(title); !ok {
					return false
				}
			}
		}
		return true
	})
	if s.ID == "CORE-LIVE-LATENCY" {
		retainLiveSamples(&evidence, effects, watchers)
	}
	for i, observer := range watchers {
		values := []float64{}
		timeouts := 0
		for title, effect := range effects {
			seen, ok := observer.observed(title)
			if !ok {
				timeouts++
				continue
			}
			values = append(values, max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond)))
		}
		name := "effect_done→gateway_visible"
		if i > 0 {
			name = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
		}
		evidence.Latency[name] = stats(values, timeouts)
	}
	add("latency_observations_complete", timeout == nil)
	for _, step := range s.Then {
		for action, options := range step {
			switch action {
			case "gateway_revoke":
			case "converge":
			case "source_ready":
				var status contracts.ReadResult
				_, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", struct{}{}, &status)
				ready := false
				for _, r := range status.Records {
					var data map[string]any
					json.Unmarshal(r.Data, &data)
					if data["source"] == "reminders" && data["state"] == "ready" {
						ready = true
					}
				}
				add("source_ready", err == nil && ready)
			case "latency":
				limit, samples := number(options, "p95_ms", 1000), number(options, "samples", 200)
				for name, latency := range evidence.Latency {
					add("latency_"+name, latency.Samples >= samples && latency.Timeouts == 0 && latency.P95 < float64(limit))
				}
			case "sse_resume":
				for i, endpoint := range endpoints {
					add(fmt.Sprintf("sse_full_cursor_resume_%d", i), sseResume(ctx, w, endpoint))
				}
			case "sse_idle":
				for i, endpoint := range endpoints {
					add(fmt.Sprintf("sse_idle_heartbeats_%d", i), sseIdle(ctx, w, endpoint))
				}
			case "capture_safety":
				add("capture_stale_noop_and_absence_protection", captureSafety(ctx, w))
			case "old_sse_cursor":
				for _, endpoint := range w.Replicas {
					status, _ := world.Request(ctx, endpoint, "/v1/changes?after=0", nil, nil)
					add("old_sse_cursor_reset", status == 409)
				}
			case "resumed":
				for _, e := range w.Replicas {
					status, err := w.Status(e)
					add("resume_without_snapshot", err == nil && status.Mode == "resume" && status.Resumes >= 1 && status.Snapshots == 1)
				}
			default:
				add("known_then_action", false)
			}
		}
	}
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		add("post_assertion_convergence", false)
	}
	recordIntegrity(&evidence, w, nil)
	for i, observer := range watchers {
		add(fmt.Sprintf("sse_contiguous_watermarks_%d", i), observer.valid())
		truth, err := w.Inspect()
		aliases, aliasErr := gatewayAliases(w)
		absent := map[string]bool{}
		for _, o := range truth {
			if o.Kind == "absent" {
				absent[o.Collection+"/"+aliases[aliasKey(contracts.CollectionSource(o.Collection), o.Collection, o.NativeKey)]] = true
			}
		}

		observer.mu.Lock()
		valid := err == nil && aliasErr == nil && !observer.falseDelete
		for _, seen := range observer.changes {
			if seen.Change.Op == "delete" && !absent[key(seen.Change.Record)] {
				valid = false
			}
		}
		observer.mu.Unlock()
		add(fmt.Sprintf("no_false_deletion_%d", i), valid)
	}
	for i, observer := range scoped {
		observer.mu.Lock()
		advanced := observer.last.Sequence > observer.initialSequence
		observer.mu.Unlock()
		add(fmt.Sprintf("scope_sse_watermarks_%d", i), observer.valid() && advanced)
	}
	for i, observer := range watchers {
		if i == 0 {
			continue
		}
		values := []float64{}
		timeouts := 0
		for title, effect := range partitionEffects {
			seen, ok := observer.observed(title)
			if !ok {
				timeouts++
				continue
			}
			values = append(values, max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond)))
		}
		if len(partitionEffects) > 0 {
			evidence.Latency[fmt.Sprintf("partition_effect_done→replica_visible:replica%d", i-1)] = stats(values, timeouts)
			add(fmt.Sprintf("partition_observations_complete_%d", i), timeouts == 0)
		}
	}

	if s.ID == "CORE-GATEWAY-DIALS" {
		gatewayRevoke(ctx, w, &evidence)
	}
	evidence.Passed = true
	for _, a := range evidence.Assertions {
		if !a.Passed && a.Status != "unverified" {
			evidence.Passed = false
		}
	}
	return evidence
}
