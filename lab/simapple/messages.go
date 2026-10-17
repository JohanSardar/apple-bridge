package simapple

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type messageState struct {
	generation string
	rearmed    int
	permitted  bool
}

func messageReference(collection, key string) map[string]any {
	return map[string]any{"$ref": map[string]string{"collection": "messages." + collection, "native_key": key}}
}

func (f *fakeApple) messageRoutes(mux *http.ServeMux) {
	if f.messages.generation == "" {
		f.messages = messageState{generation: f.generation, permitted: true}
	}
	mux.HandleFunc("GET /messages-status", func(w http.ResponseWriter, r *http.Request) {
		f.mu.Lock()
		defer f.mu.Unlock()
		json.NewEncoder(w).Encode(map[string]any{"generation": f.messages.generation, "rearmed": f.messages.rearmed, "permitted": f.messages.permitted})
	})
	mux.HandleFunc("POST /messages", func(w http.ResponseWriter, r *http.Request) {
		var action struct {
			Action string `json:"action"`
			Count  int    `json:"count"`
			Key    string `json:"key"`
		}
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&action) != nil || action.Count < 0 || action.Count > 20000 {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		now := time.Now().UTC()
		put := func(collection, key string, data map[string]any) {
			raw, _ := json.Marshal(data)
			f.records[key] = contracts.Observation{Kind: "upsert", Collection: "messages." + collection, NativeKey: key, ScopedAlias: "messages." + collection + "/" + key, ObservedAt: now, Data: raw}
		}
		message := func(key string, fromMe bool) {
			put("message", key, map[string]any{"title": "[ab-test] " + key, "chat_id": messageReference("chat", "message-chat"), "sender_handle_id": messageReference("handle", "message-self"), "is_from_me": fromMe, "sent_at": now, "text": "[ab-test] Unicode 🧪 שלום e\u0301", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": "[ab-test] Unicode 🧪 שלום e\u0301"}}, "decode_status": "decoded", "edit_history": []any{}, "retracted_parts": []any{}, "error_code": 0, "kind": "message"})
		}
		var sourceState *contracts.IPCFrame
		capture := contracts.Capture{Source: contracts.Messages}
		switch action.Action {
		case "seed":
			put("handle", "message-self", map[string]any{"title": "[ab-test] self", "address": "self@example.com", "service": "iMessage"})
			put("chat", "message-chat", map[string]any{"title": "[ab-test] chat", "service": "iMessage", "style": "direct", "display_name": "Bridge Test", "participants": []any{messageReference("handle", "message-self")}, "capabilities": []string{}})
			for i := 0; i < action.Count; i++ {
				message(fmt.Sprintf("message-%06d", i), true)
			}
			capture.ScanID = contracts.NewID("scan")
			capture.Scope = []string{"messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"}
			capture.ScanStartedAt = now
			capture.CompletionEvidence = "complete"
		case "incoming":
			if action.Key == "" {
				f.mu.Unlock()
				http.Error(w, "unavailable", 503)
				return
			}
			message(action.Key+"-outgoing", true)
			message(action.Key, false)
			capture.Observations = []contracts.Observation{f.records[action.Key+"-outgoing"], f.records[action.Key]}
		case "delete":
			old, ok := f.records[action.Key]
			if !ok {
				f.mu.Unlock()
				http.Error(w, "not found", 404)
				return
			}
			old.Kind = "absent"
			old.Revalidated = true
			old.ObservedAt = now
			f.records[action.Key] = old
			capture.ScanID = contracts.NewID("scan")
			capture.Scope = []string{"messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"}
			capture.ScanStartedAt = now
			capture.CompletionEvidence = "complete"
		case "receipts", "malformed":
			old, ok := f.records[action.Key]
			if !ok {
				f.mu.Unlock()
				http.Error(w, "not found", 404)
				return
			}
			var data map[string]any
			json.Unmarshal(old.Data, &data)
			if action.Action == "receipts" {
				data["delivered_at"] = now
				data["read_at"] = now
			} else {
				delete(data, "text")
				data["parts"] = []any{}
				data["decode_status"] = "undecoded"
				data["raw_body_digest"] = "a50c70b58b846b7ac9a6f6bbf92c4b58b6a5346202994187ad0d83346607636c"
			}
			put("message", action.Key, data)
			capture.Observations = []contracts.Observation{f.records[action.Key]}
		case "db-replacement", "schema-change":
			f.messages.generation = contracts.NewID("messages")
			capture.ScanID = contracts.NewID("scan")
			capture.Scope = []string{"messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"}
			capture.ScanStartedAt = now
			capture.CompletionEvidence = "complete"
		case "watcher-rearm":
			f.messages.rearmed++
			if action.Key != "" {
				message(action.Key, false)
				capture.Observations = []contracts.Observation{f.records[action.Key]}
			}
		case "permission-loss", "permission-restore":
			f.messages.permitted = action.Action == "permission-restore"
			state := "permission_missing"
			if f.messages.permitted {
				state = "ready"
			}
			sourceState = &contracts.IPCFrame{T: "SourceState", Source: contracts.Messages, State: state, Coverage: map[contracts.Source]contracts.Coverage{contracts.Messages: {Fields: "partial", Missing: []string{"simulated_permission_change"}}}, Capabilities: map[contracts.Source][]string{contracts.Messages: {}}}
		default:
			f.mu.Unlock()
			http.Error(w, "invalid action", 400)
			return
		}
		if err := f.saveLocked(); err != nil {
			f.mu.Unlock()
			http.Error(w, "unavailable", 503)
			return
		}
		if action.Action == "permission-restore" {
			capture.ScanID = contracts.NewID("scan")
			capture.Scope = []string{"messages.chat", "messages.handle", "messages.message", "messages.reaction", "messages.attachment"}
			capture.ScanStartedAt = now
			capture.CompletionEvidence = "complete"
		}
		if !f.messages.permitted && action.Action != "permission-loss" {
			f.mu.Unlock()
			json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
			return
		}
		if capture.ScanID != "" {
			for _, o := range f.records {
				if contracts.CollectionSource(o.Collection) == contracts.Messages {
					capture.Observations = append(capture.Observations, o)
				}
			}
		}
		capture.Generation = f.messages.generation
		f.mu.Unlock()
		if sourceState != nil {
			if err := f.messageStateFrame(r.Context(), *sourceState); err != nil {
				http.Error(w, "unavailable", 503)
				return
			}
		}
		if len(capture.Observations) == 0 && capture.ScanID == "" {
			json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
			return
		}
		if err := f.enqueue(r.Context(), capture, false); err != nil {
			http.Error(w, "unavailable", 503)
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}

func (f *fakeApple) revalidateMessages(c contracts.Capture) contracts.Capture {
	if c.Source != contracts.Messages {
		return c
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	for i, o := range c.Observations {
		if strings.HasPrefix(o.Collection, "messages.") {
			if fresh, ok := f.records[o.NativeKey]; ok {
				c.Observations[i] = fresh
			}
		}
	}
	return c
}

func messageAliases(c contracts.Capture) contracts.Capture {
	if c.Source != contracts.Messages {
		return c
	}
	aliases := map[string]contracts.Observation{}
	var collect func(any)
	collect = func(value any) {
		switch v := value.(type) {
		case map[string]any:
			if ref, ok := v["$ref"].(map[string]any); ok {
				collection, _ := ref["collection"].(string)
				key, _ := ref["native_key"].(string)
				aliases[collection+"/"+key] = contracts.Observation{Kind: "alias", Collection: collection, NativeKey: key, ScopedAlias: collection + "/" + key, ObservedAt: time.Now().UTC()}
			} else {
				for _, child := range v {
					collect(child)
				}
			}
		case []any:
			for _, child := range v {
				collect(child)
			}
		}
	}
	for _, o := range c.Observations {
		var value any
		json.Unmarshal(o.Data, &value)
		collect(value)
	}
	observations := make([]contracts.Observation, 0, len(aliases)+len(c.Observations))
	for _, o := range aliases {
		observations = append(observations, o)
	}
	c.Observations = append(observations, c.Observations...)
	return c
}

func (f *fakeApple) messageStateFrame(ctx context.Context, frame contracts.IPCFrame) error {
	select {
	case f.liveChanges <- captureRequest{SourceState: &frame}:
		return nil
	case <-ctx.Done():
		return ctx.Err()
	}
}
