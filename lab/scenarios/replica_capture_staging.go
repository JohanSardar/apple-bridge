package scenarios

import (
	"context"
	"database/sql"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func captureSQL(endpoint world.Endpoint, query string) int64 {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(endpoint.Home, "state.db")+"?mode=ro&_pragma=busy_timeout(5000)")
	if err != nil {
		return -1
	}
	defer db.Close()
	var value int64
	if db.QueryRow(query).Scan(&value) != nil {
		return -1
	}
	return value
}
func captureLegacyJournal(endpoint world.Endpoint) error {
	db, err := sql.Open("sqlite", "file:"+filepath.Join(endpoint.Home, "state.db"))
	if err != nil {
		return err
	}
	defer db.Close()
	_, err = db.Exec("DROP TABLE journal_commits")
	return err
}
func runReplicaCaptureStaging(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("staging"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Metrics: map[string]any{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	cases := []string{"upgrade-crash", "reset"}
	if s.ID == "CORE-REPLICA-CAPTURE-LEGACY" {
		cases = []string{"legacy"}
	}
	for _, name := range cases {
		add := func(label string, ok bool) {
			e.Assertions = append(e.Assertions, Assertion{Name: name + "/" + label, Passed: ok})
		}
		w, err := world.New(ctx, "sim", 1, 0)
		add("world", err == nil)
		if err != nil {
			return
		}
		captureStagingCase(ctx, w, name, &e, add)
		w.Close()
	}
	return
}
func captureStagingCase(ctx context.Context, w *world.World, name string, e *Evidence, add func(string, bool)) {
	if w.Await(15*time.Second, func() bool { return convergence(w) }) != nil {
		add("initial_convergence", false)
		return
	}
	endpoint := w.Replicas[0]
	if name == "legacy" {
		w.StopReplica(0)
		err := os.WriteFile(filepath.Join(endpoint.Home, "lab-capture-legacy"), nil, 0600)
		if err == nil {
			err = w.RestartReplica(0)
		}
		add("old_protocol_replica_started", err == nil)
		if err != nil {
			return
		}
	}
	if err := w.ProbeCaptures(name == "reset"); err != nil {
		add("frame_probe", false)
		return
	}
	w.Partition(0)
	before, _ := w.Cursor(endpoint)
	beforeRecords, readErr := w.Records(endpoint)
	add("visible_baseline", readErr == nil)
	if readErr != nil {
		return
	}
	count := 80
	for n := 0; n < count; n++ {
		data := map[string]any{"title": fmt.Sprintf("[ab-test capture] %d", n), "notes": strings.Repeat("c", 60000)}
		if err := w.ControlAction("/external-change", map[string]any{"key": fmt.Sprintf("capture-%d", n), "data": data}); err != nil {
			add("backlog_created", false)
			return
		}
		if err := w.FlushCaptures(5 * time.Second); err != nil {
			add("backlog_flushed", false)
			return
		}
	}
	target, _ := w.Cursor(w.Gateway)
	w.StopGateway()
	upgradeErr := captureLegacyJournal(w.Gateway)
	if upgradeErr == nil {
		upgradeErr = w.RestartGateway()
	}
	add("upgrade_coalesces_retained_legacy_history", upgradeErr == nil)
	if upgradeErr != nil {
		return
	}
	if name == "upgrade-crash" {
		if err := w.ArmBoundary(endpoint, "capture-part"); err != nil {
			add("stage_boundary_armed", false)
			return
		}
	}
	w.Heal(0)
	if name == "upgrade-crash" {
		reached := w.AwaitBoundary(endpoint, "capture-part", "partial_staged") == nil
		add("partial_durable_on_disk", reached && captureSQL(endpoint, "SELECT COUNT(*) FROM pending_changes") > 0)
		current, _ := w.Cursor(endpoint)
		add("partial_does_not_advance_visible_cursor", current == before)
		visible, readErr := w.Records(endpoint)
		add("partial_staging_is_absent_from_ordinary_reads", readErr == nil && len(visible) == len(beforeRecords))
		e.Metrics["upgrade-crash_staged_rows_at_kill"] = captureSQL(endpoint, "SELECT COUNT(*) FROM pending_changes")
		e.Metrics["upgrade-crash_staged_bytes_at_kill"] = captureSQL(endpoint, "SELECT COALESCE(SUM(length(payload)),0) FROM pending_changes")
		if !reached {
			return
		}
		w.CrashReplica(0)
		w.Partition(0)
		err := w.RestartReplica(0)
		add("restart_while_ingress_partitioned", err == nil)
		if err != nil {
			return
		}
		current, _ = w.Cursor(endpoint)
		add("staging_cleared_before_hello", captureSQL(endpoint, "SELECT COUNT(*) FROM pending_changes") == 0 && current == before)
		w.Heal(0)
	}
	err := w.Await(30*time.Second, func() bool {
		c, _ := w.Cursor(endpoint)
		return c.Epoch == target.Epoch && c.Sequence >= target.Sequence && convergence(w)
	})
	add("backlog_converges", err == nil && convergence(w))
	probe := w.CaptureProbe()
	status, _ := w.Status(endpoint)
	e.Metrics[name+"_partial_frames"] = probe.PartialFrames
	e.Metrics[name+"_partial_bytes"] = probe.PartialBytes
	if name == "legacy" {
		add("feature_not_accepted_for_old_replica", probe.LegacyOffers > 0 && probe.AtomicWelcomes == 0 && probe.LegacyWelcomes > 0)
		add("old_protocol_receives_no_partial_frames", probe.PartialFrames == 0 && probe.PartialFields == 0)
		return
	}
	add("capture_atomic_negotiated", probe.AtomicOffers > 0 && probe.AtomicWelcomes > 0)
	add("staging_empty_after_install", captureSQL(endpoint, "SELECT COUNT(*) FROM pending_changes") == 0)
	if name == "upgrade-crash" {
		add("partial_credits_do_not_ack_uncommitted_cursor", probe.PartialAckWithoutCursor > 0 && probe.PartialAckWithCursor == 0)
		add("oversized_backlog_streams_beyond_credit_window", probe.PartialFrames > 1 && probe.PartialBytes > contracts.ByteCredit)
		add("resume_starts_at_capture_beginning", len(probe.HelloSequences) > 0 && probe.HelloSequences[len(probe.HelloSequences)-1] == before.Sequence)
		query := fmt.Sprintf("SELECT COUNT(*) FROM applied_batches WHERE after_seq=%d AND through_seq>=%d", before.Sequence, target.Sequence)
		add("legacy_backlog_applied_in_one_streamed_transaction", captureSQL(endpoint, query) == 1)
		err = capturePruneHistory(ctx, w, target.Sequence)
		e.Metrics["retention_target"] = target.Sequence
		e.Metrics["retention_floor"] = captureSQL(w.Gateway, "SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor'")
		add("retention_advances_on_commit_boundary", err == nil && captureSQL(w.Gateway, "SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor'") == int64(target.Sequence))
		add("retention_prunes_commit_end_rows", err == nil && captureSQL(w.Gateway, "SELECT COUNT(*) FROM journal_commits WHERE through_seq<=(SELECT CAST(value AS INTEGER) FROM meta WHERE key='retention_floor')") == 0)
	} else {
		_, err := os.Stat(filepath.Join(endpoint.Home, "lab-capture-discarded"))
		add("reset_discards_pending_capture", probe.ResetFrames == 1 && err == nil)
		add("resnapshot_never_interleaves_partial_apply", status.Snapshots >= 2 && captureSQL(endpoint, "SELECT COUNT(*) FROM pending_changes") == 0)
	}
}

func capturePruneHistory(ctx context.Context, w *world.World, through uint64) error {
	w.StopGateway()
	db, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db"))
	if err != nil {
		return err
	}
	_, err = db.Exec("UPDATE journal SET committed_at=unixepoch()-3600 WHERE seq<=?", through)
	db.Close()
	if err != nil {
		return err
	}
	store, err := state.Open(w.Gateway.Home)
	if err != nil {
		return err
	}
	defer store.Close()
	_, err = store.CommitCapture(ctx, contracts.Capture{Source: contracts.Reminders, Generation: "retention-fixture", Checkpoint: "1", Observations: []contracts.Observation{{Kind: "upsert", Collection: "reminders.reminder", NativeKey: "retention-marker", ObservedAt: time.Now().UTC(), Data: []byte(`{"title":"[ab-test capture] retention marker"}`)}}})
	if err != nil {
		return err
	}
	return store.Prune(ctx, state.Retention{Rows: 1, Age: time.Second})
}
