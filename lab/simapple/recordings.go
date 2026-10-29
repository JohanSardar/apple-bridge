package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net/http"
	"strings"
	"time"
)

func (f *fakeApple) recordingsControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /recordings", func(w http.ResponseWriter, r *http.Request) {
		var request struct {
			Collection string          `json:"collection"`
			Key        string          `json:"key"`
			Data       json.RawMessage `json:"data"`
		}
		if json.NewDecoder(r.Body).Decode(&request) != nil || request.Key == "" || (request.Collection != "voice_memos.recording" && request.Collection != "voicemail.message") || len(contracts.CheckShape(request.Collection, request.Data)) != 0 {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		observation := contracts.Observation{Kind: "upsert", Collection: request.Collection, NativeKey: request.Key, ScopedAlias: request.Collection + "/fixture/" + request.Key, Data: request.Data, ObservedAt: time.Now().UTC()}
		f.records[request.Collection+"/"+request.Key] = observation
		source := contracts.Source(strings.Split(request.Collection, ".")[0])
		err := f.changedLocked(source)
		revision := f.durable.Revision[source]
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		if f.enqueue(r.Context(), contracts.Capture{Source: source, Observations: []contracts.Observation{observation}}, true, revision) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func init() { sourceRoutes = append(sourceRoutes, (*fakeApple).recordingsControl) }
