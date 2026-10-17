package simapple

import (
	"bytes"
	"encoding/json"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) remindersControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /complete-recurring", func(w http.ResponseWriter, r *http.Request) {
		var request struct {
			Key string `json:"key"`
		}
		if json.NewDecoder(r.Body).Decode(&request) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		original, exists := f.records[request.Key]
		var data map[string]any
		decode := json.NewDecoder(bytes.NewReader(original.Data))
		decode.UseNumber()
		if !exists || decode.Decode(&data) != nil || data["recurrence"] == nil || f.permissionMissing {
			f.mu.Unlock()
			http.Error(w, "unavailable", 409)
			return
		}
		data["completed"] = true
		original.Data, _ = json.Marshal(data)
		original.ObservedAt = time.Now().UTC()
		f.records[request.Key] = original
		next := original
		next.NativeKey = contracts.NewID("recurring")
		data["completed"] = false
		next.Data, _ = json.Marshal(data)
		f.records[next.NativeKey] = next
		if err := f.changedLocked(contracts.Reminders); err != nil {
			f.mu.Unlock()
			http.Error(w, "persist", 500)
			return
		}
		revision := f.durable.Revision[contracts.Reminders]
		f.mu.Unlock()
		if f.enqueue(r.Context(), fullScan([]contracts.Observation{original, next}), true, revision) != nil {
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
