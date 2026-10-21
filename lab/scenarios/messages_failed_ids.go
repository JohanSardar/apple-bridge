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

func checkFailedMessageIDs(ctx context.Context, w *world.World, evidence *Evidence, chatID string, add func(string, bool)) {
	run := evidence.RunID
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		label := "gateway"
		if index > 0 {
			label = "replica"
		}
		client := scripted.Client{Endpoint: endpoint}
		for _, fault := range []string{"delivery-failed", "ghost-send"} {
			add(label+"_"+fault+"_armed", w.ControlAction("/fault", map[string]any{"name": fault}) == nil)
			raw, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + run + "] " + fault})
			command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
			var result contracts.CommandRecord
			started := time.Now()
			err := client.Call(ctx, command.Op, command, &result)
			evidence.Latency[label+"_"+fault+"_submit_to_failure"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
			add(label+"_"+fault+"_failure_is_honest", err == nil && result.State == contracts.CommandDeliveryFailed)
			if fault == "ghost-send" {
				var proof struct {
					NativeKey string `json:"native_key"`
					Observed  bool   `json:"observed"`
					Reason    string `json:"reason"`
				}
				json.Unmarshal(result.Evidence, &proof)
				add(label+"_ghost_has_no_matching_native_record", proof.NativeKey == "" && !proof.Observed && proof.Reason == "empty_unjoined_sms")
				add(label+"_ghost_no_guessed_record", result.CommitSeq == 0 && len(result.ObservedRecordIDs) == 0)
			} else {
				var proof struct {
					NativeKey string `json:"native_key"`
				}
				json.Unmarshal(result.Evidence, &proof)
				aliases, problem := gatewayAliases(w)
				expected := aliases[aliasKey(contracts.Messages, "messages.message", proof.NativeKey)]
				add(label+"_concrete_failure_has_exact_id", problem == nil && expected != "" && result.CommitSeq > 0 && reflect.DeepEqual(result.ObservedRecordIDs, []string{expected}))
				var read contracts.ReadResult
				err = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: expected}, &read)
				add(label+"_concrete_failure_readable_at_commit", err == nil && len(read.Records) == 1 && read.Cursor.Sequence >= result.CommitSeq)
				if len(result.ObservedRecordIDs) == 1 {
					args, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + run + "] " + fault, "resend_of": result.ObservedRecordIDs[0]})
					resend := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
					var sent contracts.CommandRecord
					err = client.Call(ctx, resend.Op, resend, &sent)
					add(label+"_explicit_resend_targets_proven_failure", err == nil && sent.State == contracts.CommandObserved)
				}
			}
			for _, protocol := range []string{"rest", "mcp"} {
				client.Protocol = protocol
				var status contracts.CommandRecord
				err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &status)
				add(label+"_"+fault+"_"+protocol+"_status_parity", err == nil && status.CommitSeq == result.CommitSeq && reflect.DeepEqual(status.ObservedRecordIDs, result.ObservedRecordIDs))
			}
			var retry contracts.CommandRecord
			err = client.Call(ctx, command.Op, command, &retry)
			effects, problem := w.Effects()
			add(label+"_"+fault+"_retry_never_redispatches", err == nil && retry.State == result.State && reflect.DeepEqual(retry.ObservedRecordIDs, result.ObservedRecordIDs) && problem == nil && effects[command.ID] == 1)
		}
	}
}
