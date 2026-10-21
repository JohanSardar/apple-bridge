package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runUploadReplica(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("upload-replica"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Metrics: map[string]any{}, Latency: map[string]Latency{}, Hosts: []string{"air"}, NetworkPath: "paired feed in both connection directions"}
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	for _, reverse := range []bool{false, true} {
		if os.Getenv("AB_LAB_UPLOAD_DIRECTION") == "replica_dials" && reverse {
			continue
		}
		direction := "replica_dials"
		if reverse {
			direction = "gateway_dials"
		}
		add := func(name string, ok bool) {
			e.Assertions = append(e.Assertions, Assertion{Name: direction + "/" + name, Passed: ok})
		}
		faults := []string{""}
		if s.ID == "WEB-UPLOAD-REPLICA-STATUS" {
			faults = []string{"cursor", "register", "iterator"}
		}
		if s.ID == "WEB-UPLOAD-REPLICA-STATUS-LOAD" {
			faults = []string{"unrelated", "epoch"}
		}
		for _, fault := range faults {
			check := add
			if fault != "" {
				check = func(name string, ok bool) { add(fault+"/"+name, ok) }
			}
			uploadReplicaDirection(ctx, reverse, fault, &e, check)
		}
	}
	return
}
func uploadReplicaDirection(ctx context.Context, reverse bool, fault string, e *Evidence, add func(string, bool)) {
	if reverse {
		ctx = world.WithGatewayDials(ctx)
	}
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "messages:write,system:read"})
	add("scoped_paired_replica_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	c, err := uploadReplicaBrowserClient(ctx, w, add)
	if err != nil {
		return
	}
	authority, err := trust.Open(w.Replicas[0].Home)
	if err != nil {
		add("replica_authority", false)
		return
	}
	enrollment, err := authority.Enrollment()
	add("expected_connection_direction", err == nil && enrollment != nil && (enrollment.Dial != "") == reverse)
	connected := false
	for deadline := time.Now().Add(5 * time.Second); time.Now().Before(deadline); {
		status, err := w.Status(w.Replicas[0])
		if err == nil && status.State == "connected" {
			connected = true
			break
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(20 * time.Millisecond):
		}
	}
	add("feed_connected", connected)
	if !connected {
		return
	}
	data := bytes.Repeat([]byte("Bridge Test upload bytes\n"), 100000)
	if e.Scenario == "WEB-UPLOAD-REPLICA-FORWARD-RESUME" {
		data = bytes.Repeat([]byte("u"), 10<<20)
	}
	expected := sha256.Sum256(data)
	digest := hex.EncodeToString(expected[:])
	before, err := w.Effects()
	add("effect_baseline_available", err == nil)
	var status int
	var uploaded uploadResult
	if e.Scenario == "WEB-UPLOAD-REPLICA-FORWARD-RESUME" {
		status, uploaded = uploadReplicaForwardResume(w, c, data, e, add)
	} else if e.Scenario == "WEB-UPLOAD-REPLICA-RESUME" {
		status, uploaded = uploadReplicaResume(c, data, add)
	} else {
		rejected, _ := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(contracts.MessageFileLimit + 1)})
		add("replica_rejects_oversize_before_staging", rejected == 413)
		rejected, _ = uploadRequest(c, "POST", "upload", bytes.NewReader(data), map[string]string{"Upload-Digest": strings.Repeat("0", 64)})
		add("replica_verifies_browser_supplied_digest", rejected == 422)
		status, uploaded = uploadRequest(c, "POST", "upload", bytes.NewReader(data), nil)
	}
	add("replica_upload_returns_gateway_verified_ready", status == 200 && uploaded.State == "ready" && uploaded.Offset == int64(len(data)) && uploaded.Size == int64(len(data)))
	if status != 200 {
		return
	}
	gatewayBytes, err := os.ReadFile(filepath.Join(w.Gateway.Home, "blobs/sha256", digest[:2], digest[2:]))
	actual := sha256.Sum256(gatewayBytes)
	add("independent_gateway_bytes_and_digest_match", err == nil && len(gatewayBytes) == len(data) && actual == expected)
	effects, err := w.Effects()
	add("upload_alone_has_no_native_command_effect", err == nil && uploadEffectDelta(before, effects) == 0)
	socket, err := c.socket()
	add("replica_served_command_channel", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	args, _ := json.Marshal(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "[ab-test " + e.RunID + "] upload", "files": []contracts.MessageFile{{Name: "Bridge Test.txt", UploadID: uploaded.ID}}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args}
	if fault == "cursor" {
		add("command_application_delay_armed", w.DelayCommandApply(0, time.Second) == nil)
	} else if fault == "register" || fault == "iterator" {
		add("observation_failure_armed", os.WriteFile(filepath.Join(w.Replicas[0].Home, "command-observe-failure"), []byte(fault), 0600) == nil)
	}
	record, err := uploadStatusSubmit(ctx, w, socket, command, e.Scenario == "WEB-UPLOAD-REPLICA-LOST-RESPONSE", fault, reverse, e, add)
	direction := "replica_dials"
	if reverse {
		direction = "gateway_dials"
	}
	e.Metrics[direction+"_send_state"] = record.State
	e.Metrics[direction+"_send_error_type"] = fmt.Sprintf("%T", err)
	if err != nil {
		uploadSendDiagnostic(ctx, w, e, direction, command, err)
	}
	if fault != "" {
		add("terminal_status_arrives_without_reconciliation", err == nil && record.State == contracts.CommandObserved)
		_, faultErr := os.Stat(filepath.Join(w.Replicas[0].Home, "command-observe-failure"))
		if fault == "register" || fault == "iterator" {
			add("observation_failure_consumed", os.IsNotExist(faultErr))
		}
	}
	if record.State == contracts.CommandIndeterminate {
		err = contracts.ErrIndeterminate
	}
	var problem *contracts.Error
	if errors.As(err, &problem) {
		e.Metrics[direction+"_send_error_code"] = problem.Code
	}
	if record.State == contracts.CommandIndeterminate {
		add("transport_loss_keeps_original_indeterminate_command_id", record.Command.ID == command.ID && record.Error != nil && errors.Is(record.Error, contracts.ErrIndeterminate))
		record, err = uploadReconcile(ctx, c, command.ID)
		add("read_only_status_reconciliation_returns_original_command", err == nil && record.Command.ID == command.ID && record.State == contracts.CommandObserved)
	}
	add("fresh_send_observed_on_gateway", err == nil && record.State == contracts.CommandObserved)
	ref, ok := record.UploadReferences[uploaded.ID]
	add("gateway_admission_retains_verified_blob_reference", ok && ref.Digest == digest && ref.Size == int64(len(data)))
	effects, err = w.Effects()
	add("exactly_one_explicit_send_effect", err == nil && effects[command.ID] == 1 && uploadEffectDelta(before, effects) == 1)
	if len(record.ObservedRecordIDs) == 0 {
		add("immediate_replica_read_your_write", false)
		return
	}
	var result contracts.ReadResult
	_, err = world.Request(ctx, w.Replicas[0], "/v1/op/records.query", contracts.ReadQuery{ID: record.ObservedRecordIDs[0]}, &result)
	add("immediate_replica_read_your_write", err == nil && len(result.Records) == 1 && (result.Cursor.Epoch == record.Cursor.Epoch && result.Cursor.Sequence >= record.CommitSeq || fault == "epoch" && result.Cursor.Epoch > record.Cursor.Epoch))
	e.Metrics["uploaded_bytes_per_direction"] = len(gatewayBytes)
	uploadReplicaAbandon(w, c, add)
}

func uploadEffectDelta(before, after map[string]int) int {
	total := 0
	for id, count := range after {
		total += count - before[id]
	}
	return total
}
