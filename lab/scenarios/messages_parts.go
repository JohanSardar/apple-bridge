package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"reflect"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type messagePartStatus struct {
	Index       int    `json:"index"`
	Kind        string `json:"kind"`
	UploadID    string `json:"upload_id,omitempty"`
	RecordID    string `json:"record_id,omitempty"`
	Disposition string `json:"disposition"`
}

type messagePartsStatus struct {
	State     string              `json:"state"`
	CommitSeq uint64              `json:"commit_seq"`
	Cursor    contracts.Cursor    `json:"cursor"`
	IDs       []string            `json:"observed_record_ids"`
	Parts     []messagePartStatus `json:"parts"`
	History   []struct {
		State      string `json:"state"`
		Transition string `json:"transition"`
	} `json:"history"`
	Evidence struct {
		Parts []struct {
			Index       int    `json:"index"`
			Kind        string `json:"kind"`
			NativeKey   string `json:"native_key"`
			Disposition string `json:"disposition"`
		} `json:"parts"`
	} `json:"evidence"`
}

func checkMessageParts(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		label := "gateway"
		if index > 0 {
			label = "replica"
		}
		client := scripted.Client{Endpoint: endpoint}
		outgoingCount := func() (int, error) {
			var result contracts.ReadResult
			err := client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.message", Limit: 100}, &result)
			count := 0
			for _, record := range result.Records {
				var data struct {
					FromMe bool `json:"is_from_me"`
				}
				json.Unmarshal(record.Data, &data)
				if data.FromMe {
					count++
				}
			}
			add(label+"_cardinality_window_complete", err == nil && len(result.Records) < 100)
			return count, err
		}
		for _, fault := range []string{"", "part-delivery-failed", "part-no-effect"} {
			mixed := fault != ""
			prefix := label + "_complete"
			if mixed {
				prefix = label + "_" + fault
				add(prefix+"_fault_armed", w.ControlAction("/fault", map[string]any{"name": fault}) == nil)
			}
			files := []contracts.MessageFile{}
			for i := 0; i < 2; i++ {
				data := []byte("[ab-test " + evidence.RunID + "] part " + strconv.Itoa(i))
				file := contracts.MessageFile{Name: "same.txt", Data: base64.StdEncoding.EncodeToString(data)}
				var err error
				if index == 0 {
					file, err = stageMessageFile(ctx, endpoint, "same.txt", data)
				}
				add(prefix+"_upload_"+strconv.Itoa(i)+"_prepared", err == nil && (file.UploadID != "" || file.Data != ""))
				if err != nil {
					return
				}
				files = append(files, file)
			}
			raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + evidence.RunID + "] multipart", "files": files})
			command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
			before, beforeErr := outgoingCount()
			var result messagePartsStatus
			err := client.Call(ctx, command.Op, command, &result)
			wantState := "observed"
			if mixed {
				wantState = "delivery_failed"
			}
			add(prefix+"_truthful_aggregate", err == nil && result.State == wantState && result.CommitSeq > 0)
			after, afterErr := outgoingCount()
			wantRows := 3
			if fault == "part-no-effect" {
				wantRows = 2
			}
			add(prefix+"_captured_message_cardinality", beforeErr == nil && afterErr == nil && after-before == wantRows)
			add(prefix+"_three_helper_parts", len(result.Evidence.Parts) == 3)
			add(prefix+"_three_public_parts", len(result.Parts) == 3)
			compatible := func(cursor contracts.Cursor) bool {
				return result.CommitSeq > 0 && cursor.Epoch == result.Cursor.Epoch && cursor.SubscriptionRevision == result.Cursor.SubscriptionRevision && cursor.Sequence >= result.CommitSeq
			}
			aliases, problem := gatewayAliases(w)
			add(prefix+"_aliases_readable", problem == nil)
			expected := make([]messagePartStatus, 0, 3)
			seen := map[string]bool{}
			for i, part := range result.Evidence.Parts {
				id := aliases[aliasKey(contracts.Messages, "messages.message", part.NativeKey)]
				noEffect := part.Disposition == "failed_no_effect"
				add(prefix+"_unique_part_identity", part.Index == i && (noEffect && id == "" || !noEffect && id != "" && !seen[id]))
				seen[id] = true
				disposition := part.Disposition
				if disposition == "locally_observed" {
					disposition = "observed"
				}
				entry := messagePartStatus{Index: i, Kind: part.Kind, RecordID: id, Disposition: disposition}
				if i > 0 && i <= len(files) {
					entry.UploadID = files[i-1].UploadID
				}
				expected = append(expected, entry)
				if noEffect {
					continue
				}
				var read contracts.ReadResult
				err = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: id}, &read)
				add(prefix+"_part_readable_at_commit", err == nil && len(read.Records) == 1 && compatible(read.Cursor))
			}
			add(prefix+"_exact_parts_and_original_upload_order", reflect.DeepEqual(result.Parts, expected))
			add(prefix+"_legacy_primary_id_unchanged", len(expected) == 3 && reflect.DeepEqual(result.IDs, []string{expected[0].RecordID}))
			if mixed {
				add(prefix+"_only_last_part_failed", len(expected) == 3 && expected[0].Disposition == "observed" && expected[1].Disposition == "observed" && (fault == "part-delivery-failed" && expected[2].Disposition == "delivery_failed" || fault == "part-no-effect" && expected[2].Disposition == "failed_no_effect" && expected[2].RecordID == ""))
				if len(expected) == 3 {
					retryFile := files[1]
					if index == 0 {
						retryFile, err = stageMessageFile(ctx, endpoint, "same.txt", []byte("[ab-test "+evidence.RunID+"] part 1"))
						add(prefix+"_selected_retry_file_restaged", err == nil && retryFile.UploadID != "" && retryFile.UploadID != files[1].UploadID)
					}
					retryArgs := map[string]any{"chat_id": chatID, "files": []contracts.MessageFile{retryFile}}
					if expected[2].RecordID != "" {
						retryArgs["resend_of"] = expected[2].RecordID
					}
					args, _ := json.Marshal(retryArgs)
					resend := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
					var retried messagePartsStatus
					err = client.Call(ctx, resend.Op, resend, &retried)
					add(prefix+"_fresh_resend_contains_only_failed_attachment", err == nil && retried.State == "observed" && len(retried.Evidence.Parts) == 1 && retried.Evidence.Parts[0].Kind == "attachment" && len(retried.Parts) == 1 && retried.Parts[0].Index == 0 && retried.Parts[0].Kind == "attachment" && retried.Parts[0].RecordID != "" && !seen[retried.Parts[0].RecordID] && retried.Parts[0].UploadID == retryFile.UploadID)
					effects, problem := w.Effects()
					add(prefix+"_resend_does_not_repeat_original_dispatch", problem == nil && effects[command.ID] == 1 && effects[resend.ID] == 1)
				}
			}
			for _, protocol := range []string{"rest", "mcp"} {
				client.Protocol = protocol
				var status messagePartsStatus
				err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &status)
				add(prefix+"_"+protocol+"_parts_parity", err == nil && reflect.DeepEqual(status.Parts, expected))
			}
			add(prefix+"_replicas_reach_commit", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
			for _, observer := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
				for _, part := range expected {
					if part.RecordID == "" {
						continue
					}
					var read contracts.ReadResult
					err = (scripted.Client{Endpoint: observer}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: part.RecordID}, &read)
					add(prefix+"_replicated_part_at_compatible_cursor", err == nil && len(read.Records) == 1 && compatible(read.Cursor))
				}
			}
			var replay messagePartsStatus
			err = client.Call(ctx, command.Op, command, &replay)
			effects, problem := w.Effects()
			add(prefix+"_replay_preserves_parts_without_effect", err == nil && reflect.DeepEqual(replay.Parts, expected) && problem == nil && effects[command.ID] == 1)
		}
	}
	checkMessagePartDelayedCapture(ctx, w, evidence, chatID, add)
	checkMessageGhostPart(ctx, w, evidence, chatID, add)
	checkMessagePartAmbiguity(ctx, w, evidence, chatID, add)
	checkMessagePartReceiptFailure(ctx, w, evidence, chatID, add)
	checkMessagePartProgress(ctx, w, evidence, chatID, add)
	checkWebMessageParts(ctx, w, evidence, chatID, add)
	checkMessageCaptureBound(ctx, w, evidence, chatID, add)
}
