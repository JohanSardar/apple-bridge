package scenarios

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"syscall"
	"time"

	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRobust(ctx context.Context, s Scenario, level string, options map[string]any, evidence Evidence) Evidence {
	add := func(name string, ok bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: ok})
	}
	fail := func(name string) Evidence { add(name, false); return evidence }
	name, _ := options["case"].(string)
	if name == "credential-store" {
		return runCredentialStore(ctx, evidence)
	}
	if name == "cred-lifecycle" && level == "real" {
		home := filepath.Join("/tmp/ablab", fmt.Sprintf("credentials-%d", time.Now().UnixNano()))
		result, err := world.CredentialLifecycle(ctx, home)
		add("keychain_create_use_rotate_revoke", err == nil && result.Backend == "keychain" && result.Created && result.Used && result.Rotated && result.Revoked)
		evidence.Passed = err == nil && result.Created && result.Used && result.Rotated && result.Revoked
		return evidence
	}
	if level == "real" {
		return fail("real_level_requires_native_world")
	}
	replicas, seed := 1, 4
	if name == "scope-change" || name == "scope-leak" || name == "bench" {
		replicas = 0
	}
	if strings.HasPrefix(name, "snapshot-") {
		replicas = 0
		seed = 1000
	}
	if name == "doctor" && level == "replica" {
		ctx = world.WithPersistentReplicaStorage(ctx)
	}
	var w *world.World
	var err error
	if name == "upgrade" || name == "restore-old-schema" {
		w, err = world.NewLegacy(ctx, level, replicas, seed)
	} else {
		w, err = world.New(ctx, level, replicas, seed)
	}
	if err != nil {
		fmt.Println("world startup:", err)
		return fail("world_start")
	}
	defer w.Close()
	evidence.NetworkPath = w.NetworkPath
	add("feed_network_"+w.NetworkPath, true)
	if w.Await(15*time.Second, func() bool { return convergence(w) }) != nil {
		return fail("initial_convergence")
	}
	change := func(prefix string, count int) bool {
		for i := 0; i < count; i++ {
			if w.ControlAction("/external-change", map[string]string{"key": fmt.Sprintf("%s-%06d", prefix, i)}) != nil {
				return false
			}
		}
		return true
	}
	connected := func() bool { status, err := w.Status(w.Replicas[0]); return err == nil && status.State == "connected" }
	observers := []*watcher{}
	if name != "bench" && name != "pair" {
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			observer, err := watch(ctx, endpoint, false, func(record contracts.Record) bool { return truthAbsent(w, record) })
			if err != nil {
				return fail("deletion_observer")
			}
			defer observer.close()
			observers = append(observers, observer)
		}
	}
	switch name {
	case "scope-leak":
		if w.AddReplicaScope("reminders:read,messages:read,system:read") != nil {
			return fail("full_subscription_replica")
		}
		scoped, err := w.ScopedToken(w.Replicas[0], "reminders:read,system:read")
		if err != nil {
			return fail("reminders_only_replica_token")
		}
		w.Replicas[0].Token = scoped
		paths := []string{}
		raw, _ := options["paths"].([]any)
		for _, path := range raw {
			if name, ok := path.(string); ok {
				paths = append(paths, name)
			}
		}
		if len(paths) < 5 || !scopeLeak(ctx, w, paths, add) {
			return fail("all_registered_scope_paths")
		}
	case "pair":
		if !automaticHealthyChecks(ctx, w, &evidence, add) {
			return fail("automatic_sync_guarantees")
		}
		if w.PairingChecks() != nil {
			return fail("wrong_expired_replayed_code_rejected")
		}
		add("wrong_expired_replayed_code_rejected", true)
		start := time.Now()
		if w.RevokeReplica(0) != nil {
			return fail("revoke_replica")
		}
		if w.Await(time.Second, func() bool {
			status, err := w.Status(w.Replicas[0])
			return err == nil && status.State == "disconnected"
		}) != nil {
			return fail("live_replica_revoked_within_one_second")
		}
		add("live_replica_revoked_within_one_second", time.Since(start) < time.Second)
		if !change("revoked-change", 1) {
			return fail("post_revoke_change")
		}
		if w.ControlAction("/flush", struct{}{}) != nil {
			return fail("post_revoke_capture")
		}
		before, _ := w.Cursor(w.Replicas[0])
		sleep(ctx, 1500*time.Millisecond)
		after, _ := w.Cursor(w.Replicas[0])
		add("revoked_replica_no_further_data", before == after)
		evidence.Passed = true
		for _, a := range evidence.Assertions {
			if !a.Passed {
				evidence.Passed = false
			}
		}
		return evidence
	case "scope-change":
		token, err := w.ScopedToken(w.Gateway, "admin")
		if err != nil {
			return fail("gateway_admin_token")
		}
		w.Gateway.Token = token
		if !restrictedMessage(w, "scope-change-message") {
			return fail("messages_fixture")
		}
		if w.AddReplicaScope("reminders:read,messages:read,system:read") != nil {
			return fail("full_scope_replica")
		}
		if w.Await(15*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("full_scope_convergence")
		}
		original, err := w.Cursor(w.Replicas[0])
		if err != nil {
			return fail("scope_initial_cursor")
		}
		if w.ScopeReplica(0, "reminders:read,system:read") != nil {
			return fail("narrow_scope")
		}
		if w.Await(15*time.Second, func() bool {
			cursor, err := w.Cursor(w.Replicas[0])
			if err != nil || cursor.SubscriptionRevision <= original.SubscriptionRevision {
				return false
			}
			var result contracts.ReadResult
			_, err = world.Request(ctx, w.Replicas[0], "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.message"}, &result)
			return err == nil && len(result.Records) == 0
		}) != nil {
			return fail("new_revision_purged_messages")
		}
		report, doctorErr := w.Doctor(w.Replicas[0])
		physical, _ := report["records"].(map[string]any)
		add("narrowed_subscription_physically_purged_messages", doctorErr == nil && physical["messages.message"] == nil)
		narrowed, _ := w.Cursor(w.Replicas[0])
		add("scope_narrowed_purged_new_revision", true)
		if w.ScopeReplica(0, "reminders:read,messages:read,system:read") != nil {
			return fail("widen_scope")
		}
		if w.Await(15*time.Second, func() bool {
			cursor, err := w.Cursor(w.Replicas[0])
			return err == nil && cursor.SubscriptionRevision > narrowed.SubscriptionRevision && convergence(w)
		}) != nil {
			return fail("scope_backfill_convergence")
		}
		widened, _ := w.Cursor(w.Replicas[0])
		add("scope_widened_backfill_new_revision", widened.SubscriptionRevision > narrowed.SubscriptionRevision)
	case "upgrade":
		add("older_gateway_built_from_"+world.LegacyRevision, true)
		if w.ControlAction("/external-change", map[string]any{"key": "future-optional", "collection": "reminders.reminder", "data": map[string]any{"title": "[ab-test] future-optional", "optional_future": map[string]any{"counter": json.Number("9007199254740993")}}}) != nil {
			return fail("future_optional_capture")
		}
		if w.Await(10*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("optional_record_replicated")
		}
		records, err := w.Records(w.Replicas[0])
		preserved := false
		for _, record := range records {
			if strings.Contains(string(record.Data), "optional_future") && strings.Contains(string(record.Data), "9007199254740993") {
				preserved = true
			}
		}
		add("unknown_optional_data_preserved_on_replica", err == nil && preserved)
		status, err := w.Status(w.Replicas[0])
		add("newer_replica_compatible_schema_negotiates_older_gateway", err == nil && status.State == "connected" && status.Schema == 1)

	case "bench":
		completionTimeout := 5 * time.Minute
		contended := os.Getenv("APPLE_BRIDGE_LAB_CONTENDED_BENCH") == "1"
		if contended {
			completionTimeout = 15 * time.Minute
		}
		token, err := w.ScopedToken(w.Gateway, "admin")
		if err != nil {
			return fail("benchmark_token")
		}
		w.Gateway.Token = token
		if w.AddReplicaScope("messages:read,reminders:read,system:read") != nil {
			return fail("benchmark_replica")
		}
		start := time.Now()
		if w.SeedMessages(200000) != nil {
			return fail("seed_two_hundred_thousand_messages")
		}
		effect := time.Now()
		evidence.Metrics = map[string]any{"messages_target": 200000, "completion_timeout_ms": completionTimeout.Milliseconds(), "contended_run": contended, "capture_readiness": "committed_capture_barrier", "replica_readiness": "durable_cursor_then_integrity_count", "fts5_query_cache": "repeated_query_after_integrity_verification"}
		captureErr := w.FlushCaptures(completionTimeout)
		capturedAt := time.Now()
		evidence.Metrics["fixture_and_capture_ms"] = float64(capturedAt.Sub(start)) / float64(time.Millisecond)
		evidence.Metrics["effect_done_to_gateway_capture_ms"] = float64(capturedAt.Sub(effect)) / float64(time.Millisecond)
		if captureErr != nil {
			return fail("capture_two_hundred_thousand_messages")
		}
		head, headErr := w.Cursor(w.Gateway)
		if headErr != nil {
			return fail("benchmark_gateway_cursor")
		}
		if w.Await(completionTimeout, func() bool {
			cursor, err := w.Cursor(w.Replicas[0])
			return err == nil && cursor.Epoch == head.Epoch && cursor.Sequence >= head.Sequence
		}) != nil {
			return fail("replicate_two_hundred_thousand_messages")
		}
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			report, err := w.Doctor(endpoint)
			records, _ := report["records"].(map[string]any)
			if err != nil || records["messages.message"] != float64(200000) {
				return fail("benchmark_exact_message_counts")
			}
		}
		evidence.Metrics["messages"] = 200000

		latencies := []float64{}
		for i := 0; i < 100; i++ {
			start := time.Now()
			var result contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/search", contracts.ReadQuery{Text: "needle", Limit: 1000}, &result)
			if err != nil || len(result.Records) != 200 {
				return fail("fts5_query_matches")
			}
			latencies = append(latencies, float64(time.Since(start))/float64(time.Millisecond))
		}
		evidence.Latency["fts5_query_gateway"] = stats(latencies, 0)
		resources, err := w.Resources()
		if err != nil {
			return fail("benchmark_resources")
		}
		for name, value := range resources {
			evidence.Metrics[name] = value
		}
		add("two_hundred_thousand_message_capture_replication_fts5", true)

	case "snapshot-under-load", "snapshot-interrupt":
		w.DelayNetwork(10 * time.Millisecond)
		if err = w.AddReplica(); err != nil {
			return fail("replica_start")
		}
		if w.Await(10*time.Second, func() bool { status, err := w.Status(w.Replicas[0]); return err == nil && status.State == "snapshot" }) != nil {
			return fail("snapshot_in_progress")
		}
		if name == "snapshot-interrupt" {
			if w.Await(10*time.Second, func() bool {
				status, err := w.Status(w.Replicas[0])
				return err == nil && status.State == "snapshot" && status.SnapshotPages > 0
			}) != nil {
				return fail("snapshot_in_progress")
			}
			w.CrashReplica(0)
			if err = w.RestartReplica(0); err != nil {
				return fail("restart_replica")
			}
			add("replica_killed_during_snapshot", true)
		}
		overlap := false
		for i := 0; i < 1000; i++ {
			if !change(fmt.Sprintf("during-snapshot-%d", i), 1) {
				return fail("concurrent_changes")
			}
			if !overlap && i%10 == 0 {
				status, err := w.Status(w.Replicas[0])
				head, headErr := w.Cursor(w.Gateway)
				overlap = err == nil && headErr == nil && status.State == "snapshot" && head.Sequence > status.SnapshotSequence
			}
		}
		if name == "snapshot-under-load" {
			add("changes_committed_after_frozen_sequence_before_snapshot_end", overlap)
		}
		w.DelayNetwork(0)
		add("one_thousand_changes_during_bootstrap", true)
	case "doctor":
		gateway, gatewayErr := w.Doctor(w.Gateway)
		add("gateway_clock_not_applicable", gatewayErr == nil && gateway["clock_skew"] == "n/a")
		replica, replicaErr := w.Doctor(w.Replicas[0])
		if replicaErr != nil {
			return fail("replica_doctor")
		}
		skew, skewOK := replica["clock_skew_ms"].(float64)
		age, ageOK := replica["clock_sample_age_ms"].(float64)
		add("replica_authenticated_feed_clock_sample", skewOK && ageOK && age >= 0 && age < 15000 && replica["clock_method"] == "local_receive_minus_gateway_send_including_feed_transit")
		evidence.Metrics = map[string]any{"replica_clock_skew_ms": skew, "clock_sample_age_ms": age, "replica_disk_encryption": replica["disk_encryption"], "clock_method": replica["clock_method"]}
		if level == "replica" {
			evidence.Metrics["replica_storage"] = "persistent_user_cache"
			encrypted := replica["disk_encryption"]
			warnings, _ := replica["warnings"].([]any)
			add("linux_data_directory_encryption_diagnosed", encrypted == "enabled" || (encrypted == "warning_not_enabled" && len(warnings) > 0))
		}
		warned, unavailable, warningErr := w.DoctorClockWarnings(w.Replicas[0])
		add("clock_skew_warning_actionable", warningErr == nil && warned)
		add("clock_sample_unavailable_warning_actionable", warningErr == nil && unavailable)
	case "cred-lifecycle":
		result, err := w.Credentials()
		if err != nil {
			fmt.Println("credential adapter:", err)
			return fail("linux_credential_adapter")
		}
		add("credentials_"+result.Backend+"_create_use_rotate_revoke", result.Created && result.Used && result.Rotated && result.Revoked)
	case "gap-reject":
		if w.DropBatch() != nil {
			return fail("frame_proxy_start")
		}
		if w.Await(10*time.Second, connected) != nil {
			return fail("frame_proxy_connect")
		}
		if !change("dropped-batch", 1) {
			return fail("change_dropped_frame")
		}
		if w.Await(10*time.Second, func() bool { return w.DroppedFrames() == 1 }) != nil {
			return fail("one_batch_dropped")
		}
		if !change("after-gap", 10) {
			return fail("change_after_gap")
		}
		if w.Await(15*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("gap_recovery")
		}
		add("gap_rejected_connection_reopened", w.FeedConnections() >= 2)
	case "slow-consumer":
		if w.SuppressAcks(false) != nil {
			return fail("frame_proxy_start")
		}
		if w.Await(10*time.Second, func() bool { return w.FeedConnections() >= 1 && connected() }) != nil {
			return fail("frame_proxy_connect")
		}
		initial := w.FeedConnections()
		if w.SuppressAcks(true) != nil || !change("slow-consumer", 10) {
			return fail("suppress_acknowledgements")
		}
		start := time.Now()
		if w.Await(35*time.Second, func() bool { return w.FeedConnections() > initial }) != nil {
			return fail("slow_consumer_disconnected")
		}
		add("unacknowledged_consumer_disconnected_within_35_seconds", time.Since(start) <= 35*time.Second)
		w.SuppressAcks(false)
		w.Partition(0)
		w.Heal(0)
	case "retention-race":
		w.StopGateway()
		if w.RestartGateway(8) != nil {
			return fail("configure_small_retention")
		}
		if w.Await(10*time.Second, connected) != nil {
			return fail("gateway_reconnect")
		}
		w.Partition(0)
		before, err := w.Cursor(w.Replicas[0])
		if err != nil {
			return fail("cursor_before_partition")
		}
		if !change("beyond-retention", 100) {
			return fail("offline_changes")
		}
		if err = w.Await(15*time.Second, func() bool {
			report, err := w.Doctor(w.Gateway)
			floor, _ := report["floor"].(float64)
			return err == nil && uint64(floor) > before.Sequence
		}); err != nil {
			return fail("retention_passed_replica_cursor")
		}
		w.Heal(0)
		if w.Await(15*time.Second, connected) != nil {
			return fail("resnapshot_connected")
		}
		status, _ := w.Status(w.Replicas[0])
		add("reset_resnapshot_after_retention", status.Snapshots >= 2)
	case "kill-during-apply":
		for i := 0; i < 8; i++ {
			if w.ControlAction("/flush", struct{}{}) != nil || w.Await(10*time.Second, func() bool { return convergence(w) }) != nil {
				return fail("pre_apply_crash_convergence")
			}
			if w.ArmBoundary(w.Replicas[0], "apply") != nil {
				return fail("arm_apply_boundary")
			}
			if !change(fmt.Sprintf("crash-%d", i), 50) {
				return fail("changes_before_crash")
			}
			reached := w.AwaitBoundary(w.Replicas[0], "apply", "write_started_before_commit") == nil
			add(fmt.Sprintf("apply_write_before_commit_boundary_iteration_%d", i), reached)
			if !reached {
				return fail("apply_boundary_not_reached")
			}
			w.CrashReplica(0)
			if w.RestartReplica(0) != nil {
				return fail("replica_restart")
			}
			if w.Await(10*time.Second, connected) != nil {
				return fail("replica_reconnect")
			}
		}
		add("eight_replica_process_crashes", true)
	case "kill-gateway-capture":
		for i := 0; i < 8; i++ {
			if w.ArmBoundary(w.Gateway, "capture") != nil {
				return fail("arm_capture_boundary")
			}
			if !change(fmt.Sprintf("capture-crash-%d", i), 50) {
				return fail("changes_before_crash")
			}
			reached := w.AwaitBoundary(w.Gateway, "capture", "capture_committed_before_acknowledgment") == nil
			add(fmt.Sprintf("capture_before_committed_boundary_iteration_%d", i), reached)
			if !reached {
				return fail("capture_boundary_not_reached")
			}
			if w.SignalGateway(syscall.SIGKILL) != nil {
				return fail("kill_gateway")
			}
			w.StopGateway()
			if w.RestartGateway(0) != nil {
				return fail("gateway_restart")
			}
			drained := w.FlushCaptures(30*time.Second) == nil && w.Await(15*time.Second, func() bool { return convergence(w) }) == nil
			add(fmt.Sprintf("capture_replay_converged_iteration_%d", i), drained)
			if !drained {
				return fail("capture_iteration_recovery")
			}
		}
		add("eight_gateway_process_crashes", true)
		if w.ControlAction("/flush", struct{}{}) != nil {
			return fail("capture_replay_drained")
		}
		if w.Await(15*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("capture_replay_convergence")
		}
		records, readErr := w.Records(w.Gateway)
		deduplicated := readErr == nil
		for _, record := range records {
			if record.Collection != "system.source" && record.Version != 1 {
				deduplicated = false
			}
		}
		add("capture_replays_do_not_duplicate_record_versions", deduplicated)
	case "helper-restart":
		if w.ControlAction("/blob-fixture", map[string]int{"size": 1024}) != nil {
			return fail("durable_helper_blob_fixture")
		}
		if w.ControlAction("/flush", struct{}{}) != nil {
			return fail("helper_initial_checkpoint")
		}
		scans, err := w.HelperScans()
		if err != nil {
			return fail("helper_scan_count")
		}
		w.StopHelper()
		if w.Await(5*time.Second, func() bool {
			var result contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", struct{}{}, &result)
			for _, record := range result.Records {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				if data["state"] == "degraded" && data["reason"] == "helper_disconnected" {
					return err == nil
				}
			}
			return false
		}) != nil {
			return fail("helper_disconnected_degraded")
		}
		if w.RestartHelper() != nil {
			return fail("helper_restart")
		}
		if w.Await(10*time.Second, func() bool { _, err := w.Inspect(); return err == nil && convergence(w) }) != nil {
			return fail("helper_reconnected")
		}
		after, err := w.HelperScans()
		add("helper_checkpoint_resume_without_rescan", err == nil && after == scans)
		truth, inspectErr := w.Inspect()
		attachments := 0
		for _, observation := range truth {
			if observation.Collection == "messages.attachment" {
				attachments++
			}
		}
		add("helper_blob_truth_and_bytes_survive_restart", inspectErr == nil && attachments == 1 && w.HelperBlobsIntact())
		if !change("after-helper-restart", 10) {
			return fail("helper_live_capture_resumed")
		}
	case "disk-full":
		if err = w.CappedGatewayDisk(); err != nil {
			fmt.Println("capped disk:", err)
			return fail("dedicated_capped_disk")
		}
		if w.Await(10*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("capped_disk_ready")
		}
		w.FillDisk()
		if !change("disk-full", 100) {
			return fail("disk_full_external_changes")
		}
		if w.Await(15*time.Second, func() bool {
			var result contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", struct{}{}, &result)
			for _, record := range result.Records {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				if data["state"] == "degraded" && data["reason"] == "capacity" {
					return err == nil
				}
			}
			return false
		}) != nil {
			return fail("disk_full_degraded_capacity")
		}
		add("real_volume_full_capture_degraded", true)
		if w.FreeDisk() != nil {
			return fail("free_dedicated_disk")
		}
	case "restore-old-schema":
		backup := filepath.Join(w.Dir, "schema1-backup.db")
		if w.Backup(backup) != nil {
			return fail("backup_from_older_gateway_binary")
		}
		w.StopGateway()
		if w.RestartGateway(0) != nil {
			return fail("upgrade_before_command_admission")
		}
		if w.ControlAction("/flush", struct{}{}) != nil {
			return fail("upgraded_helper_capture_ready")
		}
		var lists contracts.ReadResult
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.list"}, &lists)
		if err != nil || len(lists.Records) == 0 {
			return fail("upgraded_reminder_list")
		}
		args, _ := json.Marshal(map[string]string{"list_id": lists.Records[0].ID, "title": "[ab-test] command after schema1 backup"})
		command := contracts.Command{ID: "old-schema-surviving-command", Op: "reminders.create", Args: args, WaitMS: 15000}
		var first contracts.CommandRecord
		_, err = world.Request(ctx, w.Gateway, "/v1/op/"+command.Op, command, &first)
		if err != nil || first.Disposition != contracts.DispositionLocallyObserved {
			return fail("command_after_old_schema_backup")
		}
		before, _ := w.Cursor(w.Gateway)
		w.StopGateway()
		if w.Restore(backup) != nil {
			return fail("restore_old_schema_with_newer_live_ledger")
		}
		if w.RestartGateway(0) != nil {
			return fail("migrate_restored_old_schema_before_publication")
		}
		if w.Await(15*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("old_schema_restore_convergence")
		}
		after, _ := w.Cursor(w.Gateway)
		var retried contracts.CommandRecord
		_, err = world.Request(ctx, w.Gateway, "/v1/op/"+command.Op, command, &retried)
		effects, effectsErr := w.Effects()
		add("old_schema_restore_rotates_epoch_keeps_live_ledger", before.Epoch != after.Epoch && err == nil && effectsErr == nil && effects[command.ID] == 1 && retried.Disposition == first.Disposition && retried.AdmissionEra == first.AdmissionEra)
	case "restore-epoch":
		backup := filepath.Join(w.Dir, "backup.db")
		if w.Backup(backup) != nil {
			return fail("gateway_backup")
		}
		old, err := w.Cursor(w.Gateway)
		if err != nil {
			return fail("old_epoch")
		}
		if !change("after-backup", 20) {
			return fail("changes_after_backup")
		}
		if w.Await(10*time.Second, func() bool { return convergence(w) }) != nil {
			return fail("before_restore_convergence")
		}
		w.StopGateway()
		if w.Restore(backup) != nil {
			return fail("restore_backup")
		}
		if w.RestartGateway(0) != nil {
			return fail("restart_restored_gateway")
		}
		if w.Await(15*time.Second, func() bool { return convergence(w) && connected() }) != nil {
			return fail("restore_convergence")
		}
		current, err := w.Cursor(w.Gateway)
		status, _ := w.Status(w.Replicas[0])
		add("restore_rotates_epoch_replica_resnapshots", err == nil && old.Epoch != current.Epoch && status.Snapshots >= 2)
	default:
		return fail("robustness_action_implemented_" + name)
	}
	drainStart := time.Now()
	drainErr := w.FlushCaptures(30 * time.Second)
	if evidence.Metrics == nil {
		evidence.Metrics = map[string]any{}
	}
	evidence.Metrics["recovery_final_drain_ms"] = float64(time.Since(drainStart)) / float64(time.Millisecond)
	if drainErr != nil {
		add("recovery_capture_drained_before_convergence", false)
		var timeout interface{ Timeout() bool }
		evidence.Assertions[len(evidence.Assertions)-1].Detail = fmt.Sprintf("timeout=%t context_cancelled=%t", errors.As(drainErr, &timeout) && timeout.Timeout(), ctx.Err() != nil)
		return evidence
	}
	add("recovery_capture_drained_before_convergence", true)
	if w.Await(30*time.Second, func() bool { return convergence(w) }) != nil {
		return fail("truth_gateway_replica_convergence")
	}
	add("truth_gateway_replica_convergence", true)
	if !automaticHealthyChecks(ctx, w, &evidence, add) {
		return fail("automatic_sync_guarantees")
	}
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		report, err := w.Doctor(endpoint)
		role := map[bool]string{true: "replica", false: "gateway"}[endpoint.Home != w.Gateway.Home]
		valid := err == nil && report["ok"] == true && report["exactly_once"] == true
		add("sqlite_integrity_"+role, valid)
		if !valid {
			evidence.Assertions[len(evidence.Assertions)-1].Detail = fmt.Sprintf("doctor_error=%t ok=%v exactly_once=%v private_modes=%v", err != nil, report["ok"], report["exactly_once"], report["private_modes"])
		}
	}
	for _, observer := range observers {
		observer.mu.Lock()
		falseDelete := observer.falseDelete
		observer.mu.Unlock()
		add("no_false_deletion_in_observed_journal_changes", !falseDelete)
	}
	evidence.Passed = true
	for _, assertion := range evidence.Assertions {
		if !assertion.Passed {
			evidence.Passed = false
		}
	}
	return evidence
}
