package simapple

import (
	"encoding/json"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) focusControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /focus", func(w http.ResponseWriter, r *http.Request) {
		var batch struct {
			Collection string          `json:"collection"`
			Key        string          `json:"key"`
			Data       json.RawMessage `json:"data"`
		}
		if json.NewDecoder(r.Body).Decode(&batch) != nil || batch.Collection != "focus.status" || len(contracts.CheckShape(batch.Collection, batch.Data)) != 0 || batch.Key != "current" {
			http.Error(w, "invalid", 400)
			return
		}
		o := contracts.Observation{Kind: "upsert", Collection: batch.Collection, NativeKey: batch.Key, ScopedAlias: "focus/" + batch.Collection + "/" + batch.Key, Data: batch.Data, ObservedAt: time.Now().UTC()}
		f.mu.Lock()
		f.records[batch.Collection+"/"+batch.Key] = o
		err := f.changedLocked("focus")
		revision := f.durable.Revision["focus"]
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		if f.enqueue(r.Context(), fullScanSource([]contracts.Observation{o}, "focus"), true, revision) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func init() { sourceRoutes = append(sourceRoutes, (*fakeApple).focusControl) }
