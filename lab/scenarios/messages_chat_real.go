package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runNativeChatProof(ctx context.Context, s Scenario, e Evidence, w *world.World, native string, state *sql.DB, registerChild func(string) error) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	var messageID string
	add("message_captured", w.Await(30*time.Second, func() bool {
		return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=?", native).Scan(&messageID) == nil
	}) == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	var message contracts.ReadResult
	err := client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: messageID}, &message)
	if err != nil || len(message.Records) != 1 {
		add("message_record", false)
		return e
	}
	var data struct {
		ChatID string `json:"chat_id"`
	}
	json.Unmarshal(message.Records[0].Data, &data)
	var nativeChat string
	state.QueryRowContext(ctx, "SELECT native_key FROM aliases WHERE collection='messages.chat' AND record_id=?", data.ChatID).Scan(&nativeChat)
	submit := func(op, id string, version uint64) contracts.CommandRecord {
		raw, _ := json.Marshal(map[string]any{"id": id})
		command := contracts.Command{ID: contracts.NewCommandID(), Op: op, Args: raw, ExpectedVersion: version, WaitMS: 15000}
		started := time.Now()
		var result contracts.CommandRecord
		client.Call(ctx, op, command, &result)
		w.Await(20*time.Second, func() bool {
			_, err := world.Request(ctx, w.Gateway, "/v1/op/commands.status", contracts.CommandQuery{ID: command.ID}, &result)
			return err == nil && result.Disposition != ""
		})
		e.Latency[op] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
		add(op+"_observed", result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered)
		return result
	}
	if s.ID == "MSG-READSTATE-REAL" {
		for _, op := range []string{"messages.mark_unread", "messages.mark_read"} {
			submit(op, data.ChatID, 0)
			truth, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": native, "chat_guid": nativeChat})
			expected := "unread"
			if op == "messages.mark_read" {
				expected = "read"
			}
			add(op+"_independent_native_truth", problem == nil && truth["status"] == "verified" && truth["chat_read_state"] == expected)
		}
		var chat contracts.ReadResult
		client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.chat", ID: data.ChatID}, &chat)
		if len(chat.Records) == 1 {
			submit("messages.delete_chat", data.ChatID, chat.Records[0].Version)
		} else {
			add("cleanup_chat_record", false)
		}
	} else {
		args, _ := json.Marshal(map[string]any{"chat_id": data.ChatID, "text": "[ab-test " + e.RunID + "] chat remainder"})
		remainder := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
		if registerChild(remainder.ID) != nil {
			add("remainder_cleanup_registration", false)
			return e
		}
		var created contracts.CommandRecord
		problem := client.Call(ctx, remainder.Op, remainder, &created)
		add("second_owned_message_retains_test_chat", problem == nil && (created.State == contracts.CommandObserved || created.State == contracts.CommandDelivered))
		if problem != nil || created.State != contracts.CommandObserved && created.State != contracts.CommandDelivered {
			e.Metrics["command_state"] = created.State
			e.Status = "environment-limited"
			return e
		}
		message = contracts.ReadResult{}
		problem = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: messageID}, &message)
		if problem != nil || len(message.Records) != 1 {
			add("fresh_delete_version", false)
			return e
		}
		submit("messages.delete_message", messageID, message.Records[0].Version)
		truth, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": native})
		visible := false
		rows, _ := truth["records"].([]any)
		for _, value := range rows {
			item, _ := value.(map[string]any)
			visible = visible || item["visible"] == true
		}
		add("message_deletion_independent_truth", problem == nil && truth["status"] == "verified" && !visible)
		var chat contracts.ReadResult
		client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.chat", ID: data.ChatID}, &chat)
		if len(chat.Records) != 1 {
			add("chat_record", false)
			return e
		}
		submit("messages.delete_chat", data.ChatID, chat.Records[0].Version)
		truth, problem = w.Truth("observe", map[string]any{"source": "messages", "guid": native, "chat_guid": nativeChat})
		add("chat_deletion_independent_truth", problem == nil && truth["status"] == "verified" && (truth["chat_present"] == false || truth["chat_active"] == false))
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	if !e.Passed {
		e.Status = "environment-limited"
	}
	return e
}
