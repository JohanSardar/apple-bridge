package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net/http"
	"time"
)

func (f *fakeApple) phoneControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /phone", func(w http.ResponseWriter, r *http.Request) {
		var request struct {
			Key  string          `json:"key"`
			Data json.RawMessage `json:"data"`
		}
		if json.NewDecoder(r.Body).Decode(&request) != nil || request.Key == "" || len(contracts.CheckShape("phone.call", request.Data)) != 0 {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		observation := contracts.Observation{Kind: "upsert", Collection: "phone.call", NativeKey: request.Key, ScopedAlias: "phone/fixture/" + request.Key, Data: request.Data, ObservedAt: time.Now().UTC()}
		f.records[request.Key] = observation
		err := f.changedLocked(contracts.Phone)
		revision := f.durable.Revision[contracts.Phone]
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		if f.enqueue(r.Context(), contracts.Capture{Source: contracts.Phone, Observations: []contracts.Observation{observation}}, true, revision) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func init() { sourceRoutes = append(sourceRoutes, (*fakeApple).phoneControl) }
