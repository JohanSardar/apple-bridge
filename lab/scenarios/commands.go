package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"path/filepath"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCommands(ctx context.Context, w *world.World, id string, e *Evidence, observers []*watcher) {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	client := scripted.Client{Endpoint: w.Gateway}
	if id == "CORE-CMD-LOST-RESPONSE" || id == "CORE-CMD-READ-YOUR-WRITE" || id == "CORE-CMD-CONFLICT" {
		client.Endpoint = w.Replicas[0]
	}
	records, err := w.Records(w.Gateway)
	if err != nil {
		add("fixture_read", false)
		return
	}
	var list, reminder contracts.Record
	for _, r := range records {
		if r.Collection == "reminders.list" {
			list = r
		}
		if r.Collection == "reminders.reminder" {
			reminder = r
		}
	}
	head, _ := w.Cursor(w.Gateway)
	command := func(op string, args map[string]any, version uint64) contracts.Command {
		b, _ := json.Marshal(args)
		return contracts.Command{ID: contracts.NewCommandID(), Op: op, Args: b, ExpectedVersion: version, WaitMS: 15000}
	}
	create := func(title string) contracts.Command {
		return command("reminders.create", map[string]any{"list_id": list.ID, "title": "[ab-test] " + title}, 0)
	}
	submit := func(c contracts.Command) (contracts.CommandRecord, error) {
		var r contracts.CommandRecord
		err := client.Call(ctx, c.Op, c, &r)

		return r, err
	}
	status := func(c contracts.Command, resolve bool) (contracts.CommandRecord, error) {
		var r contracts.CommandRecord
		err := client.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID, Resolve: resolve}, &r)
		return r, err
	}
	observed := func(r contracts.CommandRecord, err error) bool {
		return err == nil && r.Disposition == "locally_observed" && r.CommitSeq > 0
	}
	fault := func(name string, ms int) bool {
		return w.ControlAction("/fault", map[string]any{"name": name, "ms": ms}) == nil
	}
	count := func(c contracts.Command, n int) bool {
		effects, err := w.Effects()
		return err == nil && effects[c.ID] == n
	}
	for i, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		token, tokenErr := w.ScopedToken(endpoint, "system:read")
		endpoint.Token = token
		for _, protocol := range []string{"rest", "mcp"} {
			scoped := scripted.Client{Endpoint: endpoint, Protocol: protocol}
			var read contracts.ReadResult
			err := scoped.Call(ctx, "records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &read)
			add(fmt.Sprintf("scope_%s_query_%d", protocol, i), tokenErr == nil && err == nil && len(read.Records) == 0)
			c := create("scoped")
			err = scoped.Call(ctx, c.Op, c, nil)
			add(fmt.Sprintf("scope_%s_command_%d", protocol, i), errors.Is(err, contracts.ErrUnauthorized) && count(c, 0))
		}
	}
	switch id {
	case "CORE-CMD-ERA":
		backup := filepath.Join(w.Dir, "commands-before.db")
		add("backup_before_command", w.CommandBackup(backup) == nil)
		c := create("after-backup")
		c.ID = "caller-uuid-review-17"
		first, firstErr := submit(c)
		add("opaque_caller_id_executes", observed(first, firstErr) && count(c, 1))
		w.StopGateway()
		add("restore_older_backup", w.CommandRestore(backup, false) == nil)
		add("restart_after_restore", w.RestartGateway() == nil)
		add("restore_truth_reconverges", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
		restoredHead, _ := w.Cursor(w.Gateway)
		add("restore_rotates_epoch", restoredHead.Epoch != head.Epoch)
		retry, retryErr := submit(c)
		add("post_backup_command_recognized_without_second_effect", retryErr == nil && reflect.DeepEqual(first, retry) && count(c, 1))
		old := create("old-era-new-id")
		old.AdmissionEra = first.AdmissionEra
		_, oldErr := submit(old)
		add("older_era_cannot_admit_new_id", errors.Is(oldErr, contracts.ErrExpired) && count(old, 0))
		fresh := create("fresh-era")
		fresh.ID = "fresh.opaque:caller-17"
		current, currentErr := submit(fresh)
		add("fresh_opaque_id_new_era_once", observed(current, currentErr) && current.AdmissionEra != first.AdmissionEra && count(fresh, 1))
		_, currentErr = submit(fresh)
		add("fresh_opaque_retry_once", currentErr == nil && count(fresh, 1))
		w.StopGateway()
		add("remove_ledger", w.RemoveCommandLedger() == nil)
		add("missing_ledger_blocks_gateway", w.CommandLedgerLossBlocked())
		add("operator_accepts_ledger_loss", w.CommandRestore("", true) == nil)
		add("restart_after_accepted_ledger_loss", w.RestartGateway() == nil)
		accepted := create("accepted-loss-era")
		acceptedResult, acceptedErr := submit(accepted)
		add("accepted_loss_starts_new_era", observed(acceptedResult, acceptedErr) && acceptedResult.AdmissionEra != current.AdmissionEra && count(accepted, 1))
		for _, protocol := range []string{"rest", "mcp"} {
			client.Protocol = protocol
			generated := create("generated-" + protocol)
			generated.ID = ""
			result, err := submit(generated)
			add(protocol+"_generated_opaque_id", observed(result, err) && !strings.Contains(result.Command.ID, restoredHead.Epoch))
		}

	case "CORE-CMD-IDEMPOTENT":
		c := create("idempotent")
		first, err := submit(c)
		add("command_admitted", observed(first, err))
		canonical := c
		canonical.Args = json.RawMessage(fmt.Sprintf(`{  "title": "[ab-test] idempotent", "list_id": %q }`, list.ID))
		second, err := submit(canonical)

		add("identical_retry_returns_existing_status", err == nil && reflect.DeepEqual(first, second))
		c.Args, _ = json.Marshal(map[string]any{"list_id": list.ID, "title": "[ab-test] different"})
		_, err = submit(c)
		var problem *contracts.Error
		add("changed_args_id_reused", errors.As(err, &problem) && problem.Code == "id_reused")
		add("one_truth_effect", count(c, 1))
		other := client
		other.Endpoint.Token, _ = w.ScopedToken(w.Gateway, "reminders:write")
		var result contracts.CommandRecord
		err = other.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID}, &result)
		add("other_issuer_status_hidden", errors.Is(err, contracts.ErrNotFound))
	case "CORE-CMD-LOST-RESPONSE":
		c := create("peer-lost-response")
		add("replica_feed_lag_injected", w.DelayCommandApply(0, time.Second) == nil)
		add("peer_response_drop_armed", w.DropCommandResponse() == nil)
		recovered, recoverErr := submit(c)
		add("replica_recovers_lost_peer_response_by_status", observed(recovered, recoverErr) && count(c, 1))
		var local contracts.ReadResult
		readErr := client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &local)
		found := false
		for _, record := range local.Records {
			var data map[string]any
			json.Unmarshal(record.Data, &data)
			found = found || data["title"] == "[ab-test] peer-lost-response"
		}
		add("lost_peer_response_waits_for_local_write", readErr == nil && found && local.Cursor.Sequence >= recovered.CommitSeq)

		c = create("lost-response")
		drop := client
		drop.DropResponse = true
		err := drop.Call(ctx, c.Op, c, nil)
		add("response_lost", errors.Is(err, io.ErrUnexpectedEOF))
		r, err := status(c, false)
		add("lookup_original_id_observed", observed(r, err))
		r, err = submit(c)
		add("retry_original_id_observed", observed(r, err))
		add("one_truth_effect", count(c, 1))
	case "CORE-CMD-CRASH-AMBIGUOUS":
		c := create("crash")
		add("crash_fault_armed", fault("crash-after-effect", 0))
		r, err := submit(c)
		add("crash_returns_indeterminate", err == nil && r.Disposition == "indeterminate")
		add("helper_restarted", w.RestartHelper() == nil)
		add("recovered_capture", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
		r, err = submit(c)
		add("retry_stays_indeterminate", err == nil && r.Disposition == "indeterminate")
		r, err = status(c, true)
		add("resolve_strengthens", observed(r, err))
		add("resolve_never_redispatches", count(c, 1))
		c = create("ambiguous")
		fault("ambiguous-outcome", 0)
		r, err = submit(c)
		add("ambiguous_truthful", err == nil && r.Disposition == "indeterminate")
		r, err = status(c, true)
		add("ambiguous_resolves", observed(r, err) && count(c, 1))
	case "CORE-CMD-EXPIRY":
		c := create("expiry")
		c.ExpiresAt = time.Now().UTC().Add(150 * time.Millisecond)
		fault("pause", 400)
		r, err := submit(c)
		add("paused_command_expires", err == nil && r.Disposition == "expired")
		add("expired_no_effect", count(c, 0))
		sleep(ctx, 450*time.Millisecond)
		r, err = submit(c)
		add("expired_retry_never_dispatches", err == nil && r.Disposition == "expired" && count(c, 0))
		c = create("restart-expiry")
		c.WaitMS = 0
		c.ExpiresAt = time.Now().UTC().Add(200 * time.Millisecond)
		fault("pause", 500)
		admitted, admitErr := submit(c)
		add("zero_wait_returns_admitted", admitErr == nil && (admitted.State == "admitted" || admitted.State == "prepared"))
		w.StopGateway()
		sleep(ctx, 300*time.Millisecond)
		add("restart_gateway", w.RestartGateway() == nil)
		r, err = status(c, false)
		add("restart_rechecks_expiry", err == nil && r.Disposition == "expired" && count(c, 0))
	case "CORE-CMD-CLOCK-JUMP":
		for _, offset := range []int64{3600000, -3600000} {
			w.ClockOffset(0)
			c := create(fmt.Sprint("clock-", offset))
			c.ExpiresAt = time.Now().UTC().Add(600 * time.Millisecond)
			fault("pause", 200)
			done := make(chan struct{})
			var r contracts.CommandRecord
			var submitErr error
			go func() { r, submitErr = submit(c); close(done) }()
			sleep(ctx, 100*time.Millisecond)
			w.ClockOffset(offset)
			<-done
			add(fmt.Sprintf("wall_jump_%d_preserves_monotonic_deadline", offset), observed(r, submitErr) && count(c, 1))
			w.ClockOffset(0)
		}
		for _, ttl := range []time.Duration{-time.Hour, 2 * time.Hour} {
			c := create("skew")
			c.ExpiresAt = time.Now().UTC().Add(ttl)
			_, err := submit(c)
			add("skewed_expiry_rejected", errors.Is(err, contracts.ErrExpired) && count(c, 0))
		}
	case "CORE-CMD-OFFLINE":
		c := create("helper-offline")
		w.StopHelper()
		sleep(ctx, 100*time.Millisecond)
		start := time.Now()
		_, err := submit(c)
		add("helper_unavailable_fast", errors.Is(err, contracts.ErrUnavailable) && time.Since(start) < time.Second)
		add("helper_restarted", w.RestartHelper() == nil)
		w.StopGateway()
		sleep(ctx, 200*time.Millisecond)
		remote := scripted.Client{Endpoint: w.Replicas[0]}
		start = time.Now()
		err = remote.Call(ctx, c.Op, c, nil)
		add("replica_unavailable_fast", errors.Is(err, contracts.ErrUnavailable) && time.Since(start) < 2*time.Second)
		add("gateway_restarted", w.RestartGateway() == nil)
		add("no_offline_effect", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil && count(c, 0))
	case "CORE-CMD-CONFLICT":
		type attempt struct {
			Record  contracts.CommandRecord
			Command contracts.Command
			Error   error
		}
		start := make(chan struct{})
		finished := make(chan attempt, 2)
		for i := range 2 {
			c := command("reminders.update", map[string]any{"id": reminder.ID, "title": fmt.Sprintf("[ab-test] concurrent edit %d", i)}, reminder.Version)
			endpoint := scripted.Client{Endpoint: w.Replicas[i]}
			go func() {
				<-start
				var result contracts.CommandRecord
				err := endpoint.Call(ctx, c.Op, c, &result)
				finished <- attempt{result, c, err}
			}()
		}
		close(start)
		wins, conflicts := 0, 0
		truthCounts := true
		for range 2 {
			result := <-finished
			if observed(result.Record, result.Error) {
				wins++
				truthCounts = truthCounts && count(result.Command, 1)
			} else {
				var problem *contracts.Error
				if errors.As(result.Error, &problem) && problem.Code == "conflict" && problem.Current != nil && problem.Current.Version > reminder.Version {
					conflicts++
				}
				truthCounts = truthCounts && count(result.Command, 0)
			}
		}
		add("concurrent_same_version_exactly_one_winner", wins == 1 && conflicts == 1)
		add("concurrent_loser_has_no_truth_effect", truthCounts)
	case "REM-CONFLICT":
		c := command("reminders.update", map[string]any{"id": reminder.ID, "title": "[ab-test] first edit"}, reminder.Version)
		first, err := submit(c)
		add("first_version_wins", observed(first, err))
		if id == "CORE-CMD-CONFLICT" {
			client.Endpoint = w.Replicas[1]
		}
		c.ID = contracts.NewCommandID()
		_, err = submit(c)
		var problem *contracts.Error
		add("stale_version_conflict_with_current", errors.As(err, &problem) && problem.Code == "conflict" && problem.Current != nil && problem.Current.Version > reminder.Version)
		add("loser_has_no_effect", count(c, 0))
	case "CORE-CMD-READ-YOUR-WRITE":
		c := create("read-your-write")
		r, err := submit(c)
		add("command_observed", observed(r, err))
		var read contracts.ReadResult
		err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &read)
		found := false
		for _, record := range read.Records {
			var d map[string]any
			json.Unmarshal(record.Data, &d)
			found = found || d["title"] == "[ab-test] read-your-write"
		}
		add("local_read_at_commit_seq", err == nil && found && read.Cursor.Sequence >= r.CommitSeq)
		var captureSequence uint64
		observedCapture := w.Await(5*time.Second, func() bool {
			observers[0].mu.Lock()
			defer observers[0].mu.Unlock()
			for _, seen := range observers[0].changes {
				if seen.Change.Record.Collection == "reminders.reminder" && recordTitle(seen.Change.Record) == "[ab-test] read-your-write" {
					captureSequence = seen.Change.Sequence
					return true
				}
			}
			return false
		})
		add("commit_seq_identifies_observing_capture", observedCapture == nil && captureSequence == r.CommitSeq)
		var written contracts.Record
		for _, record := range read.Records {
			if recordTitle(record) == "[ab-test] read-your-write" {
				written = record
			}
		}
		beforeNoop, _ := w.Cursor(w.Gateway)
		noop := command("reminders.uncomplete", map[string]any{"id": written.ID}, written.Version)
		noChange, noChangeErr := submit(noop)
		add("noop_commit_seq_identifies_fresh_observation", observed(noChange, noChangeErr) && noChange.CommitSeq > beforeNoop.Sequence && noChange.Cursor.Sequence > noChange.CommitSeq && count(noop, 1))

		add("one_truth_effect", count(c, 1))
	case "CORE-REST-MCP-PARITY", "REM-CRUD":
		results := []contracts.Disposition{}
		for _, protocol := range []string{"rest", "mcp"} {
			client.Protocol = protocol
			c := create("crud-" + protocol)
			r, err := submit(c)
			add(protocol+"_create", observed(r, err))
			if err != nil || r.Disposition != "locally_observed" {
				continue
			}
			var created contracts.Record
			if w.Await(5*time.Second, func() bool {
				all, _ := w.Records(w.Gateway)
				for _, record := range all {
					var d map[string]any
					json.Unmarshal(record.Data, &d)
					if d["title"] == "[ab-test] crud-"+protocol {
						created = record
						return true
					}
				}
				return false
			}) != nil {
				add("created_visible", false)
				continue
			}
			for _, op := range []string{"update", "complete", "uncomplete", "delete"} {
				args := map[string]any{"id": created.ID}
				if op == "update" {
					args["notes"] = "[ab-test] revised"
				}
				c = command("reminders."+op, args, created.Version)
				r, err = submit(c)
				add(protocol+"_"+op, observed(r, err) && count(c, 1))
				results = append(results, r.Disposition)
				var read contracts.ReadResult
				err = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: created.Collection, ID: created.ID, IncludeDeleted: true}, &read)
				if err == nil && len(read.Records) == 1 {
					created = read.Records[0]
				} else {
					add(protocol+"_read_updated_version", false)
				}
				observations, truthErr := w.Inspect()
				truthOK := false
				for _, o := range observations {
					if o.NativeKey != "" && o.Collection == "reminders.reminder" {
						var d map[string]any
						json.Unmarshal(o.Data, &d)
						if d["title"] == "[ab-test] crud-"+protocol {
							switch op {
							case "complete":
								truthOK = d["completed"] == true
							case "uncomplete":
								truthOK = d["completed"] == false
							case "update":
								truthOK = d["notes"] == "[ab-test] revised"
							case "delete":
								truthOK = o.Kind == "absent"
							}
						}
					}
				}
				add(protocol+"_truth_"+op, truthErr == nil && truthOK)
			}
			c = command("reminders.create_list", map[string]any{"title": "[ab-test] list " + protocol}, 0)
			r, err = submit(c)
			add(protocol+"_create_list", observed(r, err) && count(c, 1))
		}
		add("rest_mcp_disposition_parity", len(results) == 8 && reflect.DeepEqual(results[:4], results[4:]))
	case "REM-OPERATION-SHAPES":
		for _, protocol := range []string{"rest", "mcp"} {
			client.Protocol = protocol
			for _, op := range []string{"create", "create_list", "complete", "uncomplete", "delete"} {
				c := command("reminders."+op, map[string]any{"id": reminder.ID, "list_id": list.ID, "title": "[ab-test] forbidden overwrite"}, reminder.Version)
				_, err := submit(c)
				add(protocol+"_"+op+"_rejects_foreign_fields", errors.Is(err, contracts.ErrInvalid) && count(c, 0))
			}
		}
	case "REM-NATIVE-LIST-RACE":
		fixtureErr := w.ControlAction("/external-change", map[string]any{"key": "other-list", "collection": "reminders.list", "data": map[string]any{"title": "[ab-test] Other List", "writable": true}})
		w.ControlAction("/flush", struct{}{})
		add("other_list_fixture", fixtureErr == nil)
		c := command("reminders.update", map[string]any{"id": reminder.ID, "title": "[ab-test] stale list edit"}, reminder.Version)
		fault("external-move-before-prepare", 0)
		_, err := submit(c)
		add("native_list_move_conflicts", errors.Is(err, contracts.ErrConflict) && count(c, 0))
		add("native_list_conflict_current_equals_truth", conflictCurrentTruth(w, reminder, err))
	case "REM-EXTERNAL-RACE":
		c := command("reminders.update", map[string]any{"id": reminder.ID, "title": "[ab-test] should not overwrite"}, reminder.Version)
		fault("external-edit-between-prepare-and-dispatch", 0)
		_, err := submit(c)
		add("external_race_rejects", errors.Is(err, contracts.ErrConflict) && count(c, 0))
		add("external_race_current_equals_truth", conflictCurrentTruth(w, reminder, err))
		all, _ := w.Records(w.Gateway)
		found := false
		for _, r := range all {
			var data map[string]any
			json.Unmarshal(r.Data, &data)
			found = found || data["title"] == "[ab-test] external race"
		}
		add("external_edit_preserved", found)
	case "REM-READONLY-LIST":
		err := w.ControlAction("/external-change", map[string]any{"key": "list", "collection": "reminders.list", "data": map[string]any{"title": "Bridge Test", "writable": false}})
		w.ControlAction("/flush", struct{}{})
		add("readonly_fixture", err == nil)
		c := create("readonly")
		_, err = submit(c)
		add("readonly_rejected_before_effect", errors.Is(err, contracts.ErrUnsupported) && count(c, 0))
		w.ControlAction("/external-change", map[string]any{"key": "other-list", "collection": "reminders.list", "data": map[string]any{"title": "[ab-test] Writable Destination", "writable": true}})
		w.ControlAction("/flush", struct{}{})
		all, _ := w.Records(w.Gateway)
		var destination string
		for _, record := range all {
			var data map[string]any
			json.Unmarshal(record.Data, &data)
			if data["title"] == "[ab-test] Writable Destination" {
				destination = record.ID
			}
		}
		c = command("reminders.update", map[string]any{"id": reminder.ID, "list_id": destination}, reminder.Version)
		_, err = submit(c)
		add("readonly_source_cannot_be_bypassed_by_move", errors.Is(err, contracts.ErrUnsupported) && count(c, 0))

	case "CORE-LOCALHOST-DEFENSE":
		for _, protocol := range []string{"rest", "mcp"} {
			for _, attack := range []string{"no-token", "bad-host", "foreign-origin"} {
				c := create("localhost")
				body, _ := json.Marshal(c)
				path := "/v1/op/" + c.Op
				if protocol == "mcp" {
					path = "/mcp"
				}
				req, _ := http.NewRequestWithContext(ctx, "POST", w.Gateway.URL+path, bytes.NewReader(body))
				req.Host = w.Gateway.Host
				req.Header.Set("Authorization", "Bearer "+w.Gateway.Token)
				switch attack {
				case "no-token":
					req.Header.Del("Authorization")
				case "bad-host":
					req.Host = "attacker.invalid"
				case "foreign-origin":
					req.Header.Set("Origin", "https://attacker.invalid")
				}
				resp, err := http.DefaultClient.Do(req)
				ok := err == nil && (resp.StatusCode == 401 || resp.StatusCode == 403)
				if resp != nil {
					resp.Body.Close()
				}
				add(protocol+"_"+attack, ok && count(c, 0))
			}
		}
	case "CORE-FORGED-DELEGATION":
		add("forged_delegation_rejected", forgedDelegation(ctx, w, list.ID))
	default:
		add("known_command_scenario", false)
	}
	w.ClockOffset(0)
	converged := w.Await(15*time.Second, func() bool { return convergence(w) }) == nil

	add("convergence_truth_gateway_replicas", converged)
	effects, err := w.Effects()
	single := err == nil
	for _, n := range effects {
		single = single && n <= 1
	}
	add("no_automatic_redispatch_truth_effect_counts", single)
}

func conflictCurrentTruth(w *world.World, before contracts.Record, err error) bool {
	var problem *contracts.Error
	if !errors.As(err, &problem) || problem.Current == nil || problem.Current.ID != before.ID || problem.Current.Version <= before.Version {
		return false
	}
	records, readErr := w.Records(w.Gateway)
	for _, record := range records {
		if record.ID == before.ID {
			return readErr == nil && reflect.DeepEqual(record, *problem.Current) && convergence(w)
		}
	}
	return false
}
