package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net/http"
	"strings"
	"time"
)

func (f *fakeApple) safariControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /safari", func(w http.ResponseWriter, r *http.Request) {
		var rows []contracts.Observation
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4<<20)).Decode(&rows) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		for _, row := range rows {
			if !strings.HasPrefix(row.Collection, "safari.") || row.NativeKey == "" {
				http.Error(w, "invalid", 400)
				return
			}
		}
		f.mu.Lock()
		for i := range rows {
			row := &rows[i]
			row.ObservedAt = time.Now().UTC()
			row.ScopedAlias = "safari/" + row.Collection + "/" + row.NativeKey
			f.records[row.Collection+"/"+row.NativeKey] = *row
		}
		err := f.changedLocked(contracts.Safari)
		revision := f.durable.Revision[contracts.Safari]
		f.mu.Unlock()
		if err != nil || f.enqueue(r.Context(), contracts.Capture{Source: contracts.Safari, Observations: rows}, true, revision) != nil {
			http.Error(w, "capture", 500)
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}
func init() { sourceRoutes = append(sourceRoutes, (*fakeApple).safariControl) }
