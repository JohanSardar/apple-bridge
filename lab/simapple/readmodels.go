package simapple

import (
	"encoding/json"
	"fmt"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) readModelRoutes(mux *http.ServeMux) {
	mux.HandleFunc("POST /readmodels", func(w http.ResponseWriter, r *http.Request) {
		var action struct {
			Action  string                  `json:"action"`
			Count   int                     `json:"count"`
			Records []contracts.Observation `json:"records"`
		}
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, contracts.MaxFrame)).Decode(&action) != nil || action.Count < 0 || action.Count > 170000 {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		now := time.Now().UTC()
		capture := contracts.Capture{Source: contracts.Messages, Generation: f.messages.generation}
		put := func(collection, key string, d map[string]any) {
			raw, _ := json.Marshal(d)
			o := contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, ScopedAlias: collection + "/" + key, ObservedAt: now, Data: raw}
			f.records[key] = o
			capture.Observations = append(capture.Observations, o)
		}
		switch action.Action {
		case "seed":
			capture.ScanID = contracts.NewID("scan")
			capture.Scope = []string{"messages.chat", "messages.message"}
			capture.ScanStartedAt = now
			capture.CompletionEvidence = "complete"
			for i := 0; i < 500; i++ {
				key := fmt.Sprintf("rm-chat-%03d", i)
				put("messages.chat", key, map[string]any{"display_name": fmt.Sprintf("Bridge Test %03d", i), "participants": []any{}, "style": "direct", "service": "iMessage"})
			}
			for i := 0; i < action.Count; i++ {
				chat := "rm-chat-000"
				if i < 499 {
					chat = fmt.Sprintf("rm-chat-%03d", i+1)
				}
				put("messages.message", fmt.Sprintf("rm-message-%06d", i), map[string]any{"chat_id": messageReference("chat", chat), "sent_at": time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC).Add(time.Duration(i/2) * time.Second), "is_from_me": true, "kind": "message", "text": fmt.Sprintf("[ab-test] synthetic %06d", i), "parts": []any{}})
			}
		case "put":
			for _, o := range action.Records {
				if contracts.CollectionSource(o.Collection) != contracts.Messages || o.NativeKey == "" || o.Kind != "upsert" {
					f.mu.Unlock()
					http.Error(w, "invalid fixture", 400)
					return
				}
				o.ObservedAt = now
				o.ScopedAlias = o.Collection + "/" + o.NativeKey
				f.records[o.NativeKey] = o
				capture.Observations = append(capture.Observations, o)
			}
		default:
			f.mu.Unlock()
			http.Error(w, "invalid action", 400)
			return
		}
		err := f.changedLocked(contracts.Messages)
		revision := f.durable.Revision[contracts.Messages]
		f.mu.Unlock()
		if err == nil {
			err = f.enqueue(r.Context(), capture, true, revision)
		}
		if err != nil {
			http.Error(w, "unavailable", 503)
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"committed": true})
	})
}
