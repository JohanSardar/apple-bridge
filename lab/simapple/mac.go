package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net/http"
	"strings"
	"time"
)

func (f *fakeApple) macControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /mac", func(w http.ResponseWriter, r *http.Request) {
		var batch struct {
			Collection string          `json:"collection"`
			Key        string          `json:"key"`
			Data       json.RawMessage `json:"data"`
		}
		if json.NewDecoder(r.Body).Decode(&batch) != nil || !strings.HasPrefix(batch.Collection, "mac.") || contracts.Collections[batch.Collection] == "" || batch.Key == "" {
			http.Error(w, "invalid", 400)
			return
		}
		o := contracts.Observation{Kind: "upsert", Collection: batch.Collection, NativeKey: batch.Key, ScopedAlias: "mac/" + batch.Collection + "/" + batch.Key, Data: batch.Data, ObservedAt: time.Now().UTC()}
		f.mu.Lock()
		f.records[batch.Collection+"/"+batch.Key] = o
		err := f.changedLocked("mac")
		revision := f.durable.Revision["mac"]
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		if f.enqueue(r.Context(), fullScanSource([]contracts.Observation{o}, "mac"), true, revision) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func init() { sourceRoutes = append(sourceRoutes, (*fakeApple).macControl) }
