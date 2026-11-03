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

func runMessagesSim(ctx context.Context, s Scenario, level string, evidence Evidence, seed, replicas int) Evidence {
	add := func(name string, ok bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: ok})
	}
	w, err := world.New(ctx, level, replicas, 0, world.BlobOptions{})
	if err != nil {
		add("world_start", false)
		return evidence
	}
	defer w.Close()
	action := func(name, key string, count int) bool {
		return w.ControlAction("/messages", map[string]any{"action": name, "key": key, "count": count}) == nil
	}
	await := func() bool { return w.Await(15*time.Second, func() bool { return messagesConverged(w) }) == nil }
	observers := []*watcher{}
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		observer, err := watch(ctx, endpoint, false, func(record contracts.Record) bool { return truthAbsent(w, record) })
		if err != nil {
			add("observe_changes", false)
			return evidence
		}
		observers = append(observers, observer)
		defer observer.close()
	}
	add("seed", action("seed", "", seed))
	if !await() {
		messageConvergenceDetails(w)
		add("history_equals_truth", false)
		return evidence
	}
	add("history_equals_truth", true)
	switch s.ID {
	case "MSG-HISTORY-IMPORT":
		importStarted := time.Now()
		loadStart := messagesHostLoad()
		beforeStats := messagesCaptureStats(w)
		add("bulk_pause_installed", action("bulk-pause", "message-004095", 0))
		add("bulk_started", action("seed", "", 4096))
		blocked := func() bool {
			response, err := w.Control.Get("http://sim/messages-status")
			if err != nil {
				return false
			}
			defer response.Body.Close()
			var status struct {
				Blocked bool `json:"bulk_blocked"`
			}
			return json.NewDecoder(response.Body).Decode(&status) == nil && status.Blocked
		}
		add("bulk_paused_before_known_target", w.Await(30*time.Second, blocked) == nil)
		add("live_mutation_during_import", action("receipts", "message-004095", 0))
		add("live_insert_during_import", action("incoming", "during-import", 0))
		add("live_visible_before_bulk_release", w.Await(5*time.Second, func() bool {
			for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
				records, err := w.Records(endpoint)
				if err != nil {
					return false
				}
				found := false
				for _, record := range records {
					if recordTitle(record) == "[ab-test] message-004095" {
						var data map[string]any
						json.Unmarshal(record.Data, &data)
						found = data["read_at"] != nil && data["delivered_at"] != nil
					}
				}
				if !found {
					return false
				}
			}
			return blocked()
		}) == nil)
		add("release_stale_bulk", action("bulk-release", "", 0))
		var gatewayComplete, replicaComplete time.Time
		feedComplete := w.Await(60*time.Second, func() bool {
			if at, ok := messagesHistorySeen(observers[0], 4098); ok {
				gatewayComplete = at
			}
			if at, ok := messagesHistorySeen(observers[1], 4098); ok {
				replicaComplete = at
			}
			return !gatewayComplete.IsZero() && !replicaComplete.IsZero()
		}) == nil
		truthStarted := time.Now()
		converged := feedComplete && messagesConverged(w)
		afterStats := messagesCaptureStats(w)
		metrics := map[string]any{"synthetic_rows": 4098, "clock": "same-host monotonic SSE receipt after gateway commit / replica apply", "host_before": loadStart, "host_after": messagesHostLoad(), "truth_verification_ms": float64(time.Since(truthStarted)) / float64(time.Millisecond)}
		if feedComplete {
			gatewaySeconds := gatewayComplete.Sub(importStarted).Seconds()
			replicaSeconds := replicaComplete.Sub(importStarted).Seconds()
			metrics["gateway_commit_visible_ms"] = gatewaySeconds * 1000
			metrics["replica_apply_visible_ms"] = replicaSeconds * 1000
			metrics["gateway_to_replica_tail_ms"] = float64(replicaComplete.Sub(gatewayComplete)) / float64(time.Millisecond)
			metrics["gateway_rows_per_second"] = 4098 / gatewaySeconds
			metrics["replica_rows_per_second"] = 4098 / replicaSeconds
		}
		for _, key := range []string{"capture_count", "capture_ack_ms", "capture_bytes", "credit_waits"} {
			metrics[key] = afterStats[key] - beforeStats[key]
		}
		evidence.Metrics = map[string]any{"history_import": metrics}
		if !converged {
			messageConvergenceDetails(w)
		}
		add("backfill_revalidated_against_truth", converged)
		for i, observer := range observers {
			observer.mu.Lock()
			liveSeen, regressed := false, false
			for _, seen := range observer.changes {
				if recordTitle(seen.Change.Record) != "[ab-test] message-004095" {
					continue
				}
				var data map[string]any
				json.Unmarshal(seen.Change.Record.Data, &data)
				newer := data["read_at"] != nil && data["delivered_at"] != nil
				regressed = regressed || liveSeen && !newer
				liveSeen = liveSeen || newer
			}
			observer.mu.Unlock()
			add(fmt.Sprintf("older_backfill_never_regresses_live_value_%d", i), liveSeen && !regressed)
		}
	case "MSG-INCOMING":
		samples := [][]float64{{}, {}, {}}
		timeouts := 0
		for i := 0; i < 32; i++ {
			id := fmt.Sprintf("incoming-%04d", i)
			if !action("incoming", id, 0) {
				timeouts++
				continue
			}
			at := time.Now()
			title := "[ab-test] " + id
			if w.Await(time.Second, func() bool {
				_, gateway := messageSeen(observers[0], title)
				_, replica := messageSeen(observers[1], title)
				return gateway && replica
			}) != nil {
				timeouts++
				continue
			}
			gateway, _ := messageSeen(observers[0], title)
			replica, _ := messageSeen(observers[1], title)
			samples[0] = append(samples[0], max(0, float64(gateway.At.Sub(at))/float64(time.Millisecond)))
			samples[1] = append(samples[1], max(0, float64(replica.At.Sub(at))/float64(time.Millisecond)))
			samples[2] = append(samples[2], max(0, float64(replica.At.Sub(gateway.At))/float64(time.Millisecond)))
		}
		for i, name := range []string{"effect_done→gateway_visible", "effect_done→replica_visible", "gateway_visible→replica_visible"} {
			evidence.Latency[name] = stats(samples[i], timeouts)
		}
		add("all_messages_under_one_second", timeouts == 0 && len(samples[1]) == 32 && stats(samples[1], timeouts).Max < 1000)
	case "MSG-RECEIPTS":
		add("receipts_mutate_existing_record", action("receipts", "message-000001", 0) && await())
		records, _ := w.Records(w.Replicas[0])
		found := false
		for _, record := range records {
			if record.Collection == "messages.message" && recordTitle(record) == "[ab-test] message-000001" {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				found = data["delivered_at"] != nil && data["read_at"] != nil
			}
		}
		add("replica_contains_both_receipts", found)
	case "MSG-DECODE":
		output, decoderErr := exec.CommandContext(ctx, "bash", "packaging/dev/check-decoders.sh").CombinedOutput()
		add("decoder_fixture_checks", decoderErr == nil && strings.TrimSpace(string(output)) == "all decoder checks passed\nall SourceSupport checks passed")
		add("malformed_observed", action("malformed", "message-000001", 0) && await())
		records, _ := w.Records(w.Replicas[0])
		found := false
		for _, record := range records {
			if record.Collection == "messages.message" && recordTitle(record) == "[ab-test] message-000001" {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				found = data["decode_status"] == "undecoded" && data["text"] == nil && data["raw_body_digest"] != ""
			}
		}
		add("undecoded_retains_digest_without_empty_text", found)
	case "MSG-DB-REPLACED":
		before, _ := w.Records(w.Gateway)
		old := messageGeneration(w)
		add("replacement", action("db-replacement", "", 0) && await())
		after, _ := w.Records(w.Gateway)
		add("generation_changed", old != "" && messageGeneration(w) != old)
		add("stable_identifiers_no_false_deletion", reflect.DeepEqual(messageNormalize(before), messageNormalize(after)) && sameMessageIDs(before, after))
	case "MSG-WATCHER-REARM":
		add("wal_rotation_followed_by_insert", action("watcher-rearm", "after-rearm", 0))
		add("poll_fallback_catches_disarmed_events", action("watcher-poll", "", 0) && await())
		output, nativeErr := exec.CommandContext(ctx, "bash", "packaging/dev/check-messages-capture.sh").CombinedOutput()
		if nativeErr != nil {
			os.Stderr.Write(output)
		}
		add("production_native_watcher_fault_checks", nativeErr == nil && strings.TrimSpace(string(output)) == "all Messages capture checks passed")
	case "MSG-PERMISSION-LOSS":
		before, _ := w.Records(w.Gateway)
		add("permission_loss", action("permission-loss", "", 0))
		add("native_absence_while_denied", action("delete", "message-000001", 0))
		add("native_insert_while_denied", action("incoming", "during-denial", 0))
		add("permission_state", w.Await(5*time.Second, func() bool {
			records, _ := w.Records(w.Gateway)
			for _, record := range records {
				if record.Collection == "system.source" {
					var data map[string]any
					json.Unmarshal(record.Data, &data)
					if data["state"] == "permission_missing" {
						return true
					}
				}
			}
			return false
		}) == nil)
		after, _ := w.Records(w.Gateway)
		add("no_false_absence", sameMessageIDs(before, after))
		add("permission_restore", action("permission-restore", "", 0) && await())
	default:
		add("known_messages_scenario", false)
	}
	add("automatic_convergence_with_truth", await())
	for i, observer := range observers {
		observer.mu.Lock()
		safe := !observer.falseDelete
		observer.mu.Unlock()
		add(fmt.Sprintf("automatic_no_false_deletion_%d", i), safe)
		add(fmt.Sprintf("automatic_transactional_feed_%d", i), observer.valid())
	}
	for i, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		token, err := w.ScopedToken(endpoint, "system:read")
		endpoint.Token = token
		var result contracts.ReadResult
		_, queryErr := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.message", Limit: 1000}, &result)
		add(fmt.Sprintf("automatic_no_scope_leak_%d", i), err == nil && queryErr == nil && len(result.Records) == 0)
	}
	evidence.Passed = len(evidence.Assertions) > 0
	for _, assertion := range evidence.Assertions {
		evidence.Passed = evidence.Passed && assertion.Passed
	}
	return evidence
}
