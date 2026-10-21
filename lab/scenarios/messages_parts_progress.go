package scenarios

import (
	"context"
	"encoding/json"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func checkMessagePartProgress(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	for _, fault := range []string{"part-progress", "part-progress-failure"} {
		checkMessagePartProgressCase(ctx, w, evidence, chatID, fault, func(name string, ok bool) { add(fault+"_"+name, ok) })
	}
}

func checkMessagePartProgressCase(ctx context.Context, w *world.World, evidence *Evidence, chatID, fault string, add func(string, bool)) {
	wantState, wantLater := "indeterminate", "indeterminate"
	if fault == "part-progress-failure" {
		wantState, wantLater = "delivery_failed", "failed_no_effect"
	}
	client := scripted.Client{Endpoint: w.Gateway}
	file, err := stageMessageFile(ctx, w.Gateway, "progress.txt", []byte("[ab-test "+evidence.RunID+"] progress"))
	add("progress_file_ready", err == nil)
	if err != nil {
		return
	}
	add("progress_fault_armed", w.ControlAction("/fault", map[string]any{"name": fault}) == nil)
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] progress", "files": []contracts.MessageFile{file}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	var initial messagePartsStatus
	err = client.Call(ctx, command.Op, command, &initial)
	add("progress_initial_expected_unresolved_parts", err == nil && initial.State == wantState && len(initial.Parts) == 2 && initial.Parts[0].Disposition == "indeterminate" && initial.Parts[1].Disposition == wantLater && initial.Parts[0].RecordID == "" && initial.Parts[1].RecordID == "")
	var resolved messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &resolved)
	aliases, problem := gatewayAliases(w)
	expected := ""
	if len(resolved.Evidence.Parts) > 0 {
		expected = aliases[aliasKey(contracts.Messages, "messages.message", resolved.Evidence.Parts[0].NativeKey)]
	}
	add("progress_earlier_part_observed_later_unchanged", err == nil && problem == nil && expected != "" && resolved.State == wantState && len(resolved.Parts) == 2 && resolved.Parts[0].Disposition == "observed" && resolved.Parts[0].RecordID == expected && resolved.Parts[1].Disposition == wantLater && resolved.Parts[1].RecordID == "")
	add("progress_same_aggregate_explicit_history_and_commit", resolved.CommitSeq > initial.CommitSeq && resolved.Cursor.Sequence > initial.Cursor.Sequence && len(resolved.History) == len(initial.History)+1 && resolved.History[len(resolved.History)-1].State == wantState && resolved.History[len(resolved.History)-1].Transition == "parts_updated")
	for _, protocol := range []string{"rest", "mcp"} {
		client.Protocol = protocol
		var read messagePartsStatus
		err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &read)
		add("progress_"+protocol+"_read_only_evidence_visible", err == nil && reflect.DeepEqual(read.Parts, resolved.Parts) && read.CommitSeq == resolved.CommitSeq)
	}
	var repeated messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &repeated)
	add("progress_identical_proof_does_not_append_history", err == nil && reflect.DeepEqual(repeated.Parts, resolved.Parts) && len(repeated.History) == len(resolved.History) && repeated.CommitSeq == resolved.CommitSeq)
	add("progress_stale_proof_armed", w.ControlAction("/fault", map[string]any{"name": "part-progress-stale"}) == nil)
	var stale messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &stale)
	add("progress_stale_unknown_never_erases_observation", err == nil && reflect.DeepEqual(stale.Parts, resolved.Parts) && len(stale.History) == len(resolved.History) && stale.CommitSeq == resolved.CommitSeq)
	effects, problem := w.Effects()
	add("progress_reconcile_never_redispatches", problem == nil && effects[command.ID] == 1)
}

func checkMessageGhostPart(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	add("ghost_part_fault_armed", w.ControlAction("/fault", map[string]any{"name": "ghost-send"}) == nil)
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] ghost part"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	var result messagePartsStatus
	err := client.Call(ctx, command.Op, command, &result)
	add("ghost_part_failure_never_invents_identity", err == nil && result.State == "delivery_failed" && result.CommitSeq == 0 && len(result.IDs) == 0 && len(result.Parts) == 1 && result.Parts[0].Disposition == "delivery_failed" && result.Parts[0].RecordID == "")
	var checked messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &checked)
	effects, problem := w.Effects()
	add("ghost_part_check_preserves_failure_without_redispatch", err == nil && checked.State == "delivery_failed" && len(checked.IDs) == 0 && reflect.DeepEqual(checked.Parts, result.Parts) && problem == nil && effects[command.ID] == 1)
}

func checkMessagePartReceiptFailure(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	file, err := stageMessageFile(ctx, w.Gateway, "receipt.txt", []byte("[ab-test "+evidence.RunID+"] receipt"))
	add("receipt_part_file_ready", err == nil)
	if err != nil {
		return
	}
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] receipt", "files": []contracts.MessageFile{file}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	var initial messagePartsStatus
	err = client.Call(ctx, command.Op, command, &initial)
	add("receipt_parts_initially_observed", err == nil && initial.State == "observed" && len(initial.Parts) == 2)
	if len(initial.Parts) != 2 {
		return
	}
	add("receipt_unbound_failure_armed", w.ControlAction("/fault", map[string]any{"name": "part-unbound-failure"}) == nil)
	var unbound messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &unbound)
	add("receipt_unbound_failure_never_borrows_observed_identity", err == nil && unbound.State == "observed" && reflect.DeepEqual(unbound.Parts, initial.Parts) && unbound.CommitSeq == initial.CommitSeq && len(unbound.History) == len(initial.History))
	add("receipt_part_failure_armed", w.ControlAction("/fault", map[string]any{"name": "part-receipt-failed"}) == nil)
	var failed messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &failed)
	add("receipt_part_strengthens_to_failed_with_same_identity", err == nil && failed.State == "delivery_failed" && len(failed.Parts) == 2 && failed.Parts[0] == initial.Parts[0] && failed.Parts[1].Disposition == "delivery_failed" && failed.Parts[1].RecordID == initial.Parts[1].RecordID && failed.CommitSeq > initial.CommitSeq)
	var captured contracts.ReadResult
	err = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: initial.Parts[1].RecordID}, &captured)
	code := 0
	if len(captured.Records) == 1 {
		var data struct {
			Error int `json:"error_code"`
		}
		json.Unmarshal(captured.Records[0].Data, &data)
		code = data.Error
	}
	add("receipt_part_captured_error_is_readable", err == nil && code == 100)
	if failed.State == "delivery_failed" && len(failed.Parts) == 2 {
		add("receipt_remaining_part_failure_armed", w.ControlAction("/fault", map[string]any{"name": "part-receipt-failed-first"}) == nil)
		var strengthened messagePartsStatus
		err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &strengthened)
		add("receipt_failed_aggregate_accepts_remaining_part_evidence", err == nil && strengthened.State == "delivery_failed" && len(strengthened.Parts) == 2 && strengthened.Parts[0].Disposition == "delivery_failed" && strengthened.Parts[0].RecordID == initial.Parts[0].RecordID && strengthened.Parts[1] == failed.Parts[1] && strengthened.CommitSeq > failed.CommitSeq && len(strengthened.History) == len(failed.History)+1 && strengthened.History[len(strengthened.History)-1].Transition == "parts_updated")
	}
	effects, problem := w.Effects()
	add("receipt_part_resolve_does_not_redispatch", problem == nil && effects[command.ID] == 1)
}

func checkMessagePartDelayedCapture(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	checkMessagePartDelayedCaptureCase(ctx, w, evidence, chatID, true, add)
	checkMessagePartDelayedCaptureCase(ctx, w, evidence, chatID, false, func(name string, ok bool) { add("no_action_"+name, ok) })
}

func checkMessagePartDelayedCaptureCase(ctx context.Context, w *world.World, evidence *Evidence, chatID string, resolve bool, add func(string, bool)) {
	add("delayed_capture_fault_armed", w.ControlAction("/fault", map[string]any{"name": "outcome-before-capture"}) == nil)
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] delayed capture"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	started := time.Now()
	var initial messagePartsStatus
	err := client.Call(ctx, command.Op, command, &initial)
	add("delayed_capture_initial_only_claims_bound_identity", err == nil && len(initial.Parts) == 1 && len(initial.Evidence.Parts) == 1 && (initial.State == "indeterminate" && len(initial.IDs) == 0 && initial.Parts[0].RecordID == "" || (initial.State == "observed" || initial.State == "delivered") && initial.Parts[0].RecordID != ""))
	if err != nil || len(initial.Evidence.Parts) != 1 {
		return
	}
	expected := ""
	err = w.Await(time.Second, func() bool {
		aliases, err := gatewayAliases(w)
		expected = aliases[aliasKey(contracts.Messages, "messages.message", initial.Evidence.Parts[0].NativeKey)]
		return err == nil && expected != ""
	})
	add("delayed_capture_exact_alias_arrives", err == nil && expected != "")
	var resolved messagePartsStatus
	if resolve {
		err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &resolved)
	} else {
		err = w.Await(time.Second-time.Since(started), func() bool {
			problem := client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &resolved)
			return problem == nil && (resolved.State == "observed" || resolved.State == "delivered")
		})
	}
	elapsed := time.Since(started)
	metric := "single_text_delayed_capture_resolve_ms"
	if !resolve {
		metric = "single_text_delayed_capture_no_action_ms"
	}
	evidence.Latency[metric] = stats([]float64{float64(elapsed) / float64(time.Millisecond)}, 0)
	add("delayed_capture_resolve_is_sent_with_exact_identity", err == nil && (resolved.State == "observed" || resolved.State == "delivered") && len(resolved.Parts) == 1 && resolved.Parts[0].RecordID == expected && len(resolved.IDs) == 1 && resolved.IDs[0] == expected && resolved.CommitSeq > 0)
	add("delayed_capture_no_prolonged_unknown", elapsed < time.Second)
	var read contracts.ReadResult
	err = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: expected}, &read)
	add("delayed_capture_read_your_write", err == nil && len(read.Records) == 1 && read.Cursor.Epoch == resolved.Cursor.Epoch && read.Cursor.Sequence >= resolved.CommitSeq)
	effects, problem := w.Effects()
	add("delayed_capture_resolve_effect_remains_one", problem == nil && effects[command.ID] == 1)
}

func checkMessageCaptureBound(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	add("capture_bound_fault_armed", w.ControlAction("/fault", map[string]any{"name": "outcome-without-capture"}) == nil)
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] missing capture"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	started := time.Now()
	var initial messagePartsStatus
	err := client.Call(ctx, command.Op, command, &initial)
	elapsed := time.Since(started)
	add("capture_bound_stops_with_honest_unknown", err == nil && initial.State == "indeterminate" && len(initial.IDs) == 0 && len(initial.Parts) == 1 && initial.Parts[0].RecordID == "" && elapsed >= 9*time.Second && elapsed < 12*time.Second)
	if err != nil || initial.State != "indeterminate" || len(initial.Evidence.Parts) != 1 {
		return
	}
	observations, problem := w.Inspect()
	add("capture_bound_later_capture_available", problem == nil)
	capture := contracts.Capture{Source: contracts.Messages}
	for _, observation := range observations {
		if contracts.CollectionSource(observation.Collection) == contracts.Messages {
			capture.Observations = append(capture.Observations, observation)
		}
	}
	add("capture_bound_later_capture_committed", w.ControlAction("/capture", capture) == nil)
	time.Sleep(350 * time.Millisecond)
	var unchanged messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &unchanged)
	add("capture_bound_does_not_follow_after_deadline", err == nil && unchanged.State == "indeterminate" && unchanged.Cursor == initial.Cursor && len(unchanged.IDs) == 0)
	var checked messagePartsStatus
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID, Resolve: true}, &checked)
	add("capture_bound_manual_check_remains_available", err == nil && (checked.State == "observed" || checked.State == "delivered") && len(checked.IDs) == 1 && len(checked.Parts) == 1 && checked.Parts[0].RecordID == checked.IDs[0])
	effects, problem := w.Effects()
	add("capture_bound_read_only_checks_effect_one", problem == nil && effects[command.ID] == 1)
}

func checkMessagePartAmbiguity(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	add("ambiguous_part_fault_armed", w.ControlAction("/fault", map[string]any{"name": "identical-manual-send"}) == nil)
	raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] ambiguous part"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	var result messagePartsStatus
	err := client.Call(ctx, command.Op, command, &result)
	add("ambiguous_part_never_overrides_multiple_candidate_evidence", err == nil && result.State == "indeterminate" && result.CommitSeq == 0 && len(result.IDs) == 0 && len(result.Parts) == 1 && result.Parts[0].Disposition == "indeterminate" && result.Parts[0].RecordID == "")
	effects, problem := w.Effects()
	add("ambiguous_part_effect_remains_one", problem == nil && effects[command.ID] == 1)
}
