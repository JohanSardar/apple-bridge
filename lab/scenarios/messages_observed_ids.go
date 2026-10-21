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

func checkMessageCommandIDs(ctx context.Context, w *world.World, evidence *Evidence, chat contracts.Record, add func(string, bool)) {
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		client := scripted.Client{Endpoint: endpoint}
		label := "gateway"
		if index > 0 {
			label = "replica"
		}
		var target contracts.Record
		for _, op := range []string{"send", "reply", "edit"} {
			args := map[string]any{"text": "[ab-test " + evidence.RunID + "] " + label + " " + op}
			version := uint64(0)
			if op == "send" {
				args["chat_id"] = chat.ID
				add(label+"_receipt_before_outcome_armed", w.ControlAction("/fault", map[string]any{"name": "receipt-before-outcome"}) == nil)
			} else {
				args["id"] = target.ID
			}
			if op == "edit" {
				version = target.Version
			}
			raw, _ := json.Marshal(args)
			command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages." + op, Args: raw, ExpectedVersion: version, WaitMS: 15000}
			var result contracts.CommandRecord
			started := time.Now()
			err := client.Call(ctx, command.Op, command, &result)
			add(label+"_"+op+"_observed", err == nil && result.State == contracts.CommandObserved && result.CommitSeq > 0)
			var proof struct {
				NativeKey string `json:"native_key"`
			}
			json.Unmarshal(result.Evidence, &proof)
			aliases, aliasErr := gatewayAliases(w)
			expected := aliases[aliasKey(contracts.Messages, "messages.message", proof.NativeKey)]
			add(label+"_"+op+"_native_alias_exists", aliasErr == nil && expected != "")
			var prior []string
			for _, protocol := range []string{"rest", "mcp"} {
				client.Protocol = protocol
				var status struct {
					IDs       []string `json:"observed_record_ids"`
					CommitSeq uint64   `json:"commit_seq"`
				}
				err := client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &status)
				add(label+"_"+op+"_"+protocol+"_exact_observed_id", err == nil && expected != "" && reflect.DeepEqual(status.IDs, []string{expected}) && status.CommitSeq == result.CommitSeq)
				if protocol == "mcp" {
					add(label+"_"+op+"_rest_mcp_parity", reflect.DeepEqual(prior, status.IDs))
				}
				prior = status.IDs
			}
			var read contracts.ReadResult
			err = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: expected}, &read)
			add(label+"_"+op+"_immediately_readable_at_commit", err == nil && len(read.Records) == 1 && read.Cursor.Sequence >= result.CommitSeq)
			evidence.Latency[label+"_"+op+"_command_to_readable_record"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
			if len(read.Records) != 1 {
				return
			}
			target = read.Records[0]
			if op == "send" {
				var data struct {
					DeliveredAt string `json:"delivered_at"`
				}
				json.Unmarshal(target.Data, &data)
				add(label+"_receipt_update_precedes_resolution", data.DeliveredAt != "" && target.Version > 1)
			}
			add(label+"_"+op+"_replicas_converge", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
			for _, replica := range w.Replicas {
				var records contracts.ReadResult
				err := (scripted.Client{Endpoint: replica}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: expected}, &records)
				add(label+"_"+op+"_replicated_observed_record", err == nil && len(records.Records) == 1 && records.Records[0].ID == expected && records.Cursor.Sequence >= result.CommitSeq)
			}
		}
	}
	client := scripted.Client{Endpoint: w.Gateway}
	add("ambiguous_send_fault_armed", w.ControlAction("/fault", map[string]any{"name": "identical-manual-send"}) == nil)
	raw, _ := json.Marshal(map[string]any{"chat_id": chat.ID, "text": "[ab-test " + evidence.RunID + "] ambiguous"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	var ambiguous struct {
		State string   `json:"state"`
		IDs   []string `json:"observed_record_ids"`
	}
	err := client.Call(ctx, command.Op, command, &ambiguous)
	add("ambiguous_send_does_not_invent_observed_ids", err == nil && ambiguous.State == "indeterminate" && len(ambiguous.IDs) == 0)
	err = client.Call(ctx, command.Op, command, &ambiguous)
	effects, effectErr := w.Effects()
	add("ambiguous_retry_is_lookup_only", err == nil && ambiguous.State == "indeterminate" && len(ambiguous.IDs) == 0 && effectErr == nil && effects[command.ID] == 1)
	checkWebCommandIDs(ctx, w, chat.ID, add)
}
