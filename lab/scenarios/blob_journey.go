package scenarios

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	_ "modernc.org/sqlite"
	"net/http"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"syscall"
	"time"
)

func blobProgress(ctx context.Context, e world.Endpoint) (blobs.BlobProgress, error) {
	var response struct {
		Replica struct {
			Blobs blobs.BlobProgress `json:"blobs"`
		} `json:"replica"`
	}
	_, err := world.Request(ctx, e, "/v1/op/sources.status", struct{}{}, &response)
	return response.Replica.Blobs, err
}
func blobBytes(ctx context.Context, e world.Endpoint, d, id string) (int, []byte, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", e.URL+"/v1/blobs/"+d+"?record="+id, nil)
	if err != nil {
		return 0, nil, err
	}
	req.Host = e.Host
	req.Header.Set("Authorization", "Bearer "+e.Token)
	client := *world.HTTP
	client.Timeout = 30 * time.Second
	resp, err := client.Do(req)
	if err != nil {
		return 0, nil, err
	}
	defer resp.Body.Close()
	b, err := io.ReadAll(resp.Body)
	return resp.StatusCode, b, err
}
func blobJourney(ctx context.Context, w *world.World, id string, rows int, evidence *Evidence, add func(string, bool)) bool {
	check := func(name string, ok bool) bool { add(name, ok); return ok }
	if id == "CORE-BLOB-REFERENCE-INDEX" {
		rows = 320
		client := *w.Control
		client.Timeout = 2 * time.Minute
		req, err := http.NewRequestWithContext(ctx, "POST", "http://sim/blob-history", strings.NewReader(fmt.Sprintf(`{"rows":%d}`, rows)))
		if !check("history_request", err == nil) {
			return false
		}
		response, err := client.Do(req)
		if !check("history_committed", err == nil) {
			return false
		}
		defer response.Body.Close()
		var result map[string]any
		err = json.NewDecoder(response.Body).Decode(&result)
		if !check("reference_capture_durable", err == nil && response.StatusCode == 200) {
			return false
		}

		records, err := w.Records(w.Gateway)
		count := 0
		for _, r := range records {
			if r.Collection == "reminders.reminder" && strings.HasPrefix(recordTitle(r), "[ab-test] blob-history-") {
				count++
			}
		}
		if !check(fmt.Sprintf("history_%d_durable_records", rows), err == nil && count == rows) {
			return false
		}
		cursor, err := w.Cursor(w.Gateway)
		if !check("history_durable_cursor", err == nil) {
			return false
		}
		if !check("history_replica_records_and_cursor_match", w.Await(15*time.Second, func() bool {
			replica, err := w.Records(w.Replicas[0])
			if err != nil || !reflect.DeepEqual(replica, records) {
				return false
			}
			after, err := w.Cursor(w.Replicas[0])
			return err == nil && after == cursor
		}) == nil) {
			return false
		}
		evidence.Metrics = map[string]any{"reference_rows": rows, "expected_references": rows / 5}
		return check("capture_and_replica_reference_protection", w.Await(5*time.Second, func() bool {
			for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
				progress, err := blobProgress(ctx, endpoint)
				if err != nil || len(progress.Entries) != rows/5 {
					return false
				}
				for _, entry := range progress.Entries {
					if entry.State != "offloaded" {
						return false
					}
				}
			}
			return true
		}) == nil)
	}
	size := 4 << 20
	if id == "CORE-BLOB-INTERRUPT" {
		size = 16 << 20
	}
	fixture := map[string]any{"size": size}
	if id == "CORE-BLOB-GATEWAY-RETRY" {
		fixture["drop_reads"] = 1
		fixture["slow_ms"] = 100
	}
	if id == "CORE-BLOB-OFFLOADED" {
		fixture["state"] = "offloaded"
	}
	if id == "CORE-BLOB-SLOW-READ" {
		fixture["slow_ms"] = 100
	}
	if !check("attachment_fixture", w.ControlAction("/blob-fixture", fixture) == nil) {
		return false
	}
	start := time.Now()
	measure := func(name string, since time.Time) {
		evidence.Latency[name] = stats([]float64{float64(time.Since(since)) / float64(time.Millisecond)}, 0)
	}
	var reconnectDone, bulkStarted time.Time

	var attachment contracts.Record
	var digest string
	if !check("attachment_metadata_visible", w.Await(10*time.Second, func() bool {
		records, err := w.Records(w.Gateway)
		for _, r := range records {
			if r.Collection == "messages.attachment" {
				attachment = r
				refs := contracts.BlobReferences(r.Data)
				if len(refs) == 1 {
					digest = refs[0].Digest
				}
			}
		}
		return err == nil && digest != ""
	}) == nil) {
		return false
	}
	if !check("metadata_convergence_with_blob_backlog", w.Await(time.Second, func() bool { return convergence(w) }) == nil) {
		return false
	}
	e := w.Replicas[0]
	if id == "CORE-BLOB-OFFLOADED" {
		return check("offloaded_bytes_unavailable_metadata_complete", w.Await(5*time.Second, func() bool {
			p, err := blobProgress(ctx, e)
			status, _, _ := blobBytes(ctx, e, digest, attachment.ID)
			return err == nil && p.Entries[digest].State == "offloaded" && status == 503 && convergence(w)
		}) == nil)
	}
	if id == "CORE-BLOB-SLOW-READ" {
		effect := time.Now()
		if !check("slow_apple_read_keeps_metadata_live", w.ControlAction("/external-change", map[string]string{"key": "slow-apple-blob"}) == nil && w.Await(time.Second, func() bool {
			records, _ := w.Records(e)
			for _, r := range records {
				if strings.Contains(string(r.Data), "slow-apple-blob") {
					return true
				}
			}
			return false
		}) == nil) {
			return false
		}
		evidence.Latency["effect_done→metadata_during_slow_read"] = stats([]float64{float64(time.Since(effect)) / float64(time.Millisecond)}, 0)
	}
	if id == "CORE-BLOB-INTERRUPT" {
		if !check("gateway_blob_present", w.Await(10*time.Second, func() bool {
			p, err := blobProgress(ctx, w.Gateway)
			return err == nil && p.Entries[digest].State == "present"
		}) == nil) {
			return false
		}
		gatewayPresent := time.Now()
		measure("effect_done→gateway_blob_present", start)
		if !check("partial_blob_persisted_before_interrupt", w.Await(15*time.Second, func() bool {
			p, err := blobProgress(ctx, e)
			n := p.Entries[digest].Received
			return err == nil && n > 0 && n < int64(size)
		}) == nil) {
			return false
		}

		measure("gateway_present→replica_partial", gatewayPresent)
		if !check("interrupted_halfway_through_binary_chunk", w.Await(10*time.Second, func() bool {
			_, err := os.Stat(filepath.Join(w.Dir, "blob-cut-window"))
			p, perr := blobProgress(ctx, e)
			return err == nil && perr == nil && p.Entries[digest].Received == contracts.BlobChunkSize
		}) == nil) {
			return false
		}

		reconnectStart := time.Now()
		w.Partition(0)
		if !check("partition_disconnect_observed", w.Await(10*time.Second, func() bool {
			status, err := w.Status(e)
			return err == nil && status.State == "disconnected"
		}) == nil) {
			return false
		}
		if !check("partition_healed", w.Heal(0) == nil) {
			return false
		}
		if !check("replica_reconnected_before_healthy_latency", w.Await(10*time.Second, func() bool {
			status, err := w.Status(e)
			return err == nil && status.State == "connected"
		}) == nil) {
			return false
		}

		if !check("reconnected_feed", w.Await(10*time.Second, func() bool {
			var status contracts.ReadResult
			_, err := world.Request(ctx, e, "/v1/op/replica.status", struct{}{}, &status)
			return err == nil && status.Replica != nil && status.Replica.State == "connected"
		}) == nil) {
			return false
		}
		reconnectDone = time.Now()
		measure("interrupt→reconnected", reconnectStart)
		p, err := blobProgress(ctx, e)
		before := p.Entries[digest].Received
		evidence.Metrics = map[string]any{"fixture_bytes": size, "feed_chunk_bytes": contracts.BlobChunkSize, "lab_delay_per_feed_chunk_ms": 150}
		if !check("active_bulk_transfer_barrier", err == nil && w.Await(3*time.Second, func() bool {
			p, err := blobProgress(ctx, e)
			n := p.Entries[digest].Received
			evidence.Metrics["bulk_barrier_received_bytes"] = n
			evidence.Metrics["bulk_barrier_outstanding_bytes"] = int64(size) - n
			return err == nil && n > before && int64(size)-n >= 8*contracts.BlobChunkSize
		}) == nil) {
			return false
		}
		bulkStarted = time.Now()
		effect := time.Now()
		if !check("live_metadata_during_bulk", w.ControlAction("/external-change", map[string]string{"key": "during-blob"}) == nil && w.Await(time.Second, func() bool {
			records, _ := w.Records(e)
			for _, r := range records {
				if strings.Contains(string(r.Data), "during-blob") {
					progress, err := blobProgress(ctx, e)
					evidence.Metrics["metadata_observed_outstanding_bytes"] = int64(size) - progress.Entries[digest].Received
					return err == nil && progress.Entries[digest].Received < int64(size) && progress.Entries[digest].State != "present"
				}
			}
			return false
		}) == nil) {
			return false
		}
		evidence.Latency["effect_done→metadata_during_blob"] = stats([]float64{float64(time.Since(effect)) / float64(time.Millisecond)}, 0)
	}
	if id == "CORE-BLOB-GATEWAY-RETRY" {
		if !check("gateway_retries_timed_out_read", w.Await(18*time.Second, func() bool {
			p, err := blobProgress(ctx, w.Gateway)
			n := p.Entries[digest].Received
			return err == nil && n > 0 && n < int64(size)
		}) == nil) {
			return false
		}
		if !check("helper_restarts_during_staged_download", w.RestartHelper() == nil) {
			return false
		}
		if !check("gateway_resumes_with_fresh_handles", w.Await(15*time.Second, func() bool {
			p, err := blobProgress(ctx, w.Gateway)
			return err == nil && p.Entries[digest].State == "present" && p.ResumedBytes > 0
		}) == nil) {
			return false
		}
	}
	if id == "CORE-BLOB-CAP" {
		if !check("cap_reports_over_cap", w.Await(10*time.Second, func() bool {
			p, err := blobProgress(ctx, e)
			return err == nil && p.Entries[digest].State == "over_cap" && p.Bytes <= 1<<20
		}) == nil) {
			return false
		}
		status, _, _ := blobBytes(ctx, e, digest, attachment.ID)
		return check("cap_keeps_metadata_denies_absent_bytes", status == 503 && convergence(w))
	}
	if !check("blob_complete", w.Await(20*time.Second, func() bool { p, err := blobProgress(ctx, e); return err == nil && p.Entries[digest].State == "present" }) == nil) {
		return false
	}
	if !reconnectDone.IsZero() {
		measure("reconnected→blob_complete", reconnectDone)
	}
	if !bulkStarted.IsZero() {
		measure("bulk_barrier→blob_complete", bulkStarted)
	}
	evidence.Latency["effect_done→blob_complete"] = stats([]float64{float64(time.Since(start)) / float64(time.Millisecond)}, 0)
	if evidence.Metrics == nil {
		evidence.Metrics = map[string]any{}
	}
	for i, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		readStart := time.Now()
		status, b, err := blobBytes(ctx, endpoint, digest, attachment.ID)
		evidence.Metrics[fmt.Sprintf("digest_check_%d_read_ms", i)] = float64(time.Since(readStart)) / float64(time.Millisecond)
		evidence.Metrics[fmt.Sprintf("digest_check_%d_bytes", i)] = len(b)
		evidence.Metrics[fmt.Sprintf("digest_check_%d_status", i)] = status
		evidence.Metrics[fmt.Sprintf("digest_check_%d_deadline_exceeded", i)] = errors.Is(err, context.DeadlineExceeded)
		evidence.Metrics[fmt.Sprintf("digest_check_%d_error_type", i)] = fmt.Sprintf("%T", err)
		sum := sha256.Sum256(b)
		if !check("authorized_bytes_digest_verified", err == nil && status == 200 && len(b) == size && hex.EncodeToString(sum[:]) == digest) {
			return false
		}
	}
	if id == "CORE-BLOB-GC" {
		w.StopGateway()
		db, err := sql.Open("sqlite", filepath.Join(w.Gateway.Home, "state.db"))
		if !check("legacy_reference_index_open", err == nil) {
			return false
		}
		_, err = db.ExecContext(ctx, "DROP TRIGGER IF EXISTS blob_refs_record_delete; DROP TRIGGER IF EXISTS blob_refs_staging_delete; DROP TABLE IF EXISTS blob_refs; DELETE FROM meta WHERE key='blob_refs_version'")
		db.Close()
		if !check("legacy_reference_index_removed", err == nil) {
			return false
		}
		if !check("reference_index_migration_restart", w.RestartGateway() == nil) {
			return false
		}
		if !check("reference_index_migration_converges", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil) {
			return false
		}
		db, err = sql.Open("sqlite", filepath.Join(w.Gateway.Home, "state.db"))
		if !check("migrated_reference_index_open", err == nil) {
			return false
		}
		var references int
		err = db.QueryRowContext(ctx, "SELECT count(*) FROM blob_refs WHERE owner=? AND digest=?", "record:"+attachment.ID, digest).Scan(&references)
		db.Close()
		if !check("legacy_reference_index_backfilled", err == nil && references == 1) {
			return false
		}
	}
	if id == "CORE-BLOB-INTERRUPT" {
		p, _ := blobProgress(ctx, e)
		if !check("transfer_resumed_from_nonzero_offset", p.ResumedBytes > 0) {
			return false
		}
	}
	if id == "CORE-BLOB-GATEWAY-RETRY" {
		failedStart := time.Now()
		if !check("retry_exhaustion_fixture", w.ControlAction("/blob-fixture", map[string]any{"size": 128 << 10, "unavailable_reads": 8, "refresh_before_unavailable": true}) == nil) {
			return false
		}
		var failedDigest string
		if !check("five_blob_read_faults_before_gateway_restart", w.Await(8*time.Second, func() bool {
			response, err := w.Control.Get("http://sim/blob-faults")
			if err != nil {
				return false
			}
			defer response.Body.Close()
			var status struct {
				Remaining int `json:"unavailable_remaining"`
			}
			if json.NewDecoder(response.Body).Decode(&status) != nil || status.Remaining != 3 {
				return false
			}
			records, err := w.Records(w.Gateway)
			for _, r := range records {
				if r.ID == attachment.ID {
					refs := contracts.BlobReferences(r.Data)
					if len(refs) == 1 && refs[0].Digest != digest {
						failedDigest = refs[0].Digest
					}
				}
			}
			return err == nil && failedDigest != ""
		}) == nil) {
			return false
		}
		before, err := blobProgress(ctx, w.Gateway)
		if !check("retry_manifest_visible_before_restart", err == nil) {
			return false
		}
		retryBefore := before.Entries[failedDigest]
		evidence.Metrics["retry_before_gateway_restart"] = retryBefore
		if !check("gateway_killed_during_retry_backoff", w.SignalGateway(syscall.SIGKILL) == nil) {
			return false
		}
		w.StopGateway()
		var persisted struct {
			Entries map[string]contracts.BlobEntry
		}
		manifestBytes, manifestErr := os.ReadFile(filepath.Join(w.Gateway.Home, "blobs", "manifest.json"))
		if manifestErr == nil {
			manifestErr = json.Unmarshal(manifestBytes, &persisted)
		}
		if !check("retry_manifest_read_after_crash", manifestErr == nil) {
			return false
		}
		durableEntry := persisted.Entries[failedDigest]
		evidence.Metrics["retry_manifest_at_gateway_crash"] = durableEntry
		if !check("gateway_restarts_during_retry_backoff", w.RestartGateway() == nil) {
			return false
		}
		after, err := blobProgress(ctx, w.Gateway)
		retryAfter := after.Entries[failedDigest]
		evidence.Metrics["retry_after_gateway_restart"] = retryAfter
		durableBudget := check("retry_budget_and_deadline_survive_gateway_restart", err == nil && durableEntry.RetryAttempts == 5 && retryAfter.RetryAttempts == 5 && !durableEntry.NextAttemptAt.IsZero() && retryAfter.NextAttemptAt.Equal(durableEntry.NextAttemptAt))
		if !check("bounded_retries_surface_terminal_reason", w.Await(22*time.Second, func() bool {
			records, err := w.Records(w.Gateway)
			for _, r := range records {
				if r.ID != attachment.ID {
					continue
				}
				refs := contracts.BlobReferences(r.Data)
				if len(refs) == 1 && refs[0].Digest != digest {
					failedDigest = refs[0].Digest
				}
			}
			p, perr := blobProgress(ctx, w.Gateway)
			entry := p.Entries[failedDigest]
			evidence.Metrics["retry_terminal_entry"] = entry
			return err == nil && perr == nil && failedDigest != "" && entry.State == "failed" && entry.Reason == "ipc_retry_exhausted"
		}) == nil) {
			return false
		}
		if !durableBudget {
			return false
		}
		if !check("retry_backoff_is_bounded", time.Since(failedStart) >= 15*time.Second && time.Since(failedStart) < 22*time.Second) {
			return false
		}
		measure("retry_exhaustion→gateway_failed", failedStart)
		if !check("replica_surfaces_terminal_failure", w.Await(time.Second, func() bool {
			p, err := blobProgress(ctx, e)
			return err == nil && p.Entries[failedDigest].State == "failed" && p.Entries[failedDigest].Reason != ""
		}) == nil) {
			return false
		}
	}
	if id == "CORE-BLOB-CORRUPT" {
		p, _ := blobProgress(ctx, e)
		if !check("corruption_detected_and_refetched", p.Corruptions == 1) {
			return false
		}
	}
	if id == "CORE-BLOB-NO-DIGEST-AUTH" {
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			status, _, _ := blobBytes(ctx, endpoint, digest, "")
			if !check("digest_alone_denied", status == 401) {
				return false
			}
			status, _, _ = blobBytes(ctx, endpoint, digest, strings.Repeat("x", 30))
			if !check("unknown_record_denied", status == 401) {
				return false
			}
			records, _ := w.Records(endpoint)
			for _, r := range records {
				if r.Collection == "reminders.list" {
					status, _, _ = blobBytes(ctx, endpoint, digest, r.ID)
					if !check("unrelated_record_denied", status == 401) {
						return false
					}
					break
				}
			}
			token, err := w.ScopedToken(endpoint, "reminders:read,system:read")
			if err != nil {
				return check("scope_token", false)
			}
			endpoint.Token = token
			scopedProgress, scopeErr := blobProgress(ctx, endpoint)
			_, leaked := scopedProgress.Entries[digest]
			if !check("out_of_scope_blob_status_denied", scopeErr == nil && !leaked && scopedProgress.Bytes == 0) {
				return false
			}
			status, _, _ = blobBytes(ctx, endpoint, digest, attachment.ID)
			if !check("out_of_scope_record_denied", status == 401) {
				return false
			}
		}
	}
	if id == "CORE-BLOB-GC" {
		path := filepath.Join(w.Gateway.Home, "blobs", "sha256", digest[:2], digest[2:])
		info, err := os.Stat(path)
		if !check("private_immutable_blob", err == nil && info.Mode().Perm() == 0600) {
			return false
		}
		if !check("referenced_blob_survives_grace", sleep(ctx, 700*time.Millisecond) == nil) {
			return false
		}
		if _, err = os.Stat(path); !check("record_protects_blob", err == nil) {
			return false
		}

		if !check("snapshot_reference_acquired", w.BlobControl(map[string]string{"action": "snapshot", "owner": "gc-snapshot"}, nil) == nil) {
			return false
		}
		var staged struct {
			Digest string `json:"digest"`
			Pin    string `json:"pin"`
		}
		if !check("stage_returns_pin", w.BlobControl(map[string]string{"action": "stage"}, &staged) == nil && staged.Pin != "") {
			return false
		}
		pinnedPath := filepath.Join(w.Gateway.Home, "blobs", "sha256", staged.Digest[:2], staged.Digest[2:])
		if !check("delete_attachment_and_message", w.ControlAction("/blob-fixture", map[string]any{"delete": true}) == nil) {
			return false
		}

		if !check("deletion_metadata_converges", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil) {
			return false
		}
		sleep(ctx, 800*time.Millisecond)
		_, snapshotErr := os.Stat(path)
		_, pinErr := os.Stat(pinnedPath)
		if !check("snapshot_only_and_pin_survive_grace", snapshotErr == nil && pinErr == nil) {
			return false
		}
		if !check("command_reference_acquired", w.BlobControl(map[string]string{"action": "command", "owner": "gc-command", "digest": digest}, nil) == nil) {
			return false
		}
		if !check("snapshot_release", w.BlobControl(map[string]string{"action": "snapshot-release", "owner": "gc-snapshot"}, nil) == nil) {
			return false
		}
		sleep(ctx, 700*time.Millisecond)
		_, err = os.Stat(path)
		if !check("command_only_survives_grace", err == nil) {
			return false
		}
		if !check("command_release", w.BlobControl(map[string]string{"action": "command-release", "owner": "gc-command"}, nil) == nil) {
			return false
		}
		sleep(ctx, 700*time.Millisecond)
		_, err = os.Stat(pinnedPath)
		if !check("uncommitted_pin_still_protects_blob", err == nil) {
			return false
		}
		if !check("staging_pin_release", w.BlobControl(map[string]string{"action": "release", "pin": staged.Pin}, nil) == nil) {
			return false
		}
		if !check("released_pin_collected", w.Await(5*time.Second, func() bool { _, err := os.Stat(pinnedPath); return os.IsNotExist(err) }) == nil) {
			return false
		}
		if !check("unreferenced_blob_collected", w.Await(10*time.Second, func() bool {
			_, err := os.Stat(path)
			p, perr := blobProgress(ctx, e)
			_, ok := p.Entries[digest]
			return os.IsNotExist(err) && perr == nil && !ok
		}) == nil) {
			return false
		}
	}
	return true
}

func convergenceFailure(w *world.World) string {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	var gateway contracts.ReadResult
	if _, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Limit: 10000}, &gateway); err != nil {
		return "gateway_read_failed"
	}
	truth, err := w.Inspect()
	if err != nil {
		return "truth_read_failed"
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return "alias_read_failed"
	}
	records, err := truthRecords(truth, aliases)
	if err != nil {
		return "truth_alias_failed"
	}
	expected, actual := recordMultiset(records, false), recordMultiset(gateway.Records, false)
	if !reflect.DeepEqual(expected, actual) {
		a, _ := json.Marshal(expected)
		b, _ := json.Marshal(actual)
		return fmt.Sprintf("truth_mismatch counts=%d/%d hashes=%x/%x", len(expected), len(actual), sha256.Sum256(a), sha256.Sum256(b))
	}
	for i, e := range w.Replicas {
		var replica contracts.ReadResult
		if _, err := world.Request(ctx, e, "/v1/op/records.query", contracts.ReadQuery{Limit: 10000}, &replica); err != nil {
			return fmt.Sprintf("replica_%d_read_failed", i)
		}
		if replica.Cursor != gateway.Cursor {
			return fmt.Sprintf("replica_%d_cursor_mismatch sequences=%d/%d revisions=%d/%d epochs_equal=%t", i, replica.Cursor.Sequence, gateway.Cursor.Sequence, replica.Cursor.SubscriptionRevision, gateway.Cursor.SubscriptionRevision, replica.Cursor.Epoch == gateway.Cursor.Epoch)
		}
		if !reflect.DeepEqual(replica.Records, gateway.Records) {
			return fmt.Sprintf("replica_%d_records_mismatch counts=%d/%d", i, len(replica.Records), len(gateway.Records))
		}
	}
	return "matched_after_timeout"
}
