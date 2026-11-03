package scenarios

import (
	"bytes"
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os/exec"
	"time"
)

func readmodelAliasContract(ctx context.Context) bool {
	w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "admin"})
	if err != nil {
		return false
	}
	defer w.Close()
	sequence := 0
	put := func(generation, previous, checkpoint, key, text string, aliases bool) error {
		sequence++
		at := time.Date(2031, 1, 1, 0, 0, sequence, 0, time.UTC)
		records := []contracts.Observation{
			rmObservation("messages.message", key, map[string]any{"chat_id": rmRef("messages.chat", key), "sender_handle_id": rmRef("messages.handle", key), "kind": "message", "text": text, "sent_at": at, "is_from_me": false}),
			rmObservation("messages.chat", key, map[string]any{"display_name": "[ab-test] alias chat", "participants": []any{}}),
			rmObservation("messages.handle", key, map[string]any{"address": "probe@example.com", "service": "iMessage"}),
		}
		for i := range records {
			records[i].ObservedAt = at
			if aliases {
				records[i].ScopedAlias = "readmodel/" + records[i].Collection
			}
		}
		err := w.ControlAction("/capture-exact", contracts.Capture{Source: contracts.Messages, Generation: generation, Predecessor: previous, Checkpoint: checkpoint, Observations: records})
		return err
	}
	read := func(collection string) ([]contracts.Record, error) {
		var result contracts.ReadResult
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, Limit: 20}, &result)
		return result.Records, err
	}
	if put("first", "", "one", "same", "[ab-test] original", true) != nil {
		return false
	}
	messages, err := read("messages.message")
	if err != nil || len(messages) != 1 {
		return false
	}
	chats, err := read("messages.chat")
	if err != nil || len(chats) != 1 {
		return false
	}
	handles, err := read("messages.handle")
	if err != nil || len(handles) != 1 {
		return false
	}
	messageID, chatID, handleID := messages[0].ID, chats[0].ID, handles[0].ID
	if messageID == chatID || chatID == handleID || messageID == handleID {
		return false
	}
	data := rmData(messages[0])
	if data["chat_id"] != chatID || data["sender_handle_id"] != handleID {
		return false
	}
	if put("second", "", "two", "renamed", "[ab-test] resumed", true) != nil {
		return false
	}
	messages, err = read("messages.message")
	if err != nil || len(messages) != 1 || messages[0].ID != messageID || messages[0].Version != 2 {
		return false
	}
	data = rmData(messages[0])
	if data["chat_id"] != chatID || data["sender_handle_id"] != handleID {
		return false
	}
	at := time.Date(2031, 1, 1, 0, 0, 3, 0, time.UTC)
	independent := rmObservation("messages.message", "same", map[string]any{"chat_id": rmRef("messages.chat", "renamed"), "sender_handle_id": rmRef("messages.handle", "renamed"), "kind": "message", "text": "[ab-test] independent", "sent_at": at, "is_from_me": false})
	independent.ObservedAt = at
	if err = w.ControlAction("/capture-exact", contracts.Capture{Source: contracts.Messages, Generation: "second", Predecessor: "two", Checkpoint: "three", Observations: []contracts.Observation{independent}}); err != nil {
		return false
	}
	messages, err = read("messages.message")
	if err != nil || len(messages) != 2 {
		return false
	}
	distinct := false
	for _, record := range messages {
		if rmData(record)["text"] == "[ab-test] independent" {
			distinct = record.ID != messageID
		}
	}
	if !distinct {
		return false
	}
	alias := contracts.Observation{Kind: "alias", Collection: "messages.message", NativeKey: "alias-only", ScopedAlias: "readmodel/messages.message", ObservedAt: at.Add(time.Second)}
	updated := independent
	updated.NativeKey = "alias-only"
	updated.ScopedAlias = alias.ScopedAlias
	updated.ObservedAt = alias.ObservedAt
	fields := rmData(messages[0])
	fields["chat_id"] = rmRef("messages.chat", "renamed")
	fields["sender_handle_id"] = rmRef("messages.handle", "renamed")
	fields["text"] = "[ab-test] alias update"
	updated.Data = mustJSON(fields)
	if err = w.ControlAction("/capture-exact", contracts.Capture{Source: contracts.Messages, Generation: "second", Predecessor: "three", Checkpoint: "four", Observations: []contracts.Observation{alias, updated}}); err != nil {
		return false
	}
	messages, err = read("messages.message")
	if err != nil || len(messages) != 2 {
		return false
	}
	for _, record := range messages {
		if rmData(record)["text"] == "[ab-test] alias update" {
			return record.ID == messageID && record.Version == 3
		}
	}
	return false
}

func runCaptureAliases(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	passed := readmodelAliasContract(ctx)
	return Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: string(bytes.TrimSpace(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Passed: passed, Assertions: []Assertion{{Name: "capture_alias_scopes_generations_and_forward_references", Passed: passed}}, Conditions: map[string]string{"dataset": "synthetic contacts-free Messages identity fixture", "transport": "Selected gateway Helper IPC capture and REST queries"}}
}
