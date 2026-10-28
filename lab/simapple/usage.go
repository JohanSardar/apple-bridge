package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"math"
	"net/http"
	"sort"
	"time"
)

type usageEvent struct {
	At       float64 `json:"at"`
	Starting bool    `json:"starting"`
	Bundle   string  `json:"bundle"`
}

func (f *fakeApple) usageControl(mux *http.ServeMux) {
	events := map[string][]usageEvent{}
	mux.HandleFunc("POST /usage", func(w http.ResponseWriter, r *http.Request) {
		var batch struct {
			Device   string       `json:"device"`
			Platform string       `json:"platform"`
			Events   []usageEvent `json:"events"`
		}
		if json.NewDecoder(r.Body).Decode(&batch) != nil || batch.Device == "" {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		events[batch.Device] = append(events[batch.Device], batch.Events...)
		sorted := append([]usageEvent{}, events[batch.Device]...)
		sort.Slice(sorted, func(i, j int) bool {
			if sorted[i].At != sorted[j].At {
				return sorted[i].At < sorted[j].At
			}
			if sorted[i].Starting != sorted[j].Starting {
				return !sorted[i].Starting
			}
			return sorted[i].Bundle < sorted[j].Bundle
		})
		observations := []contracts.Observation{}
		add := func(collection, key string, data map[string]any) {
			raw, _ := json.Marshal(data)
			o := contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, ScopedAlias: "usage/" + collection + "/" + key, Data: raw, ObservedAt: time.Now().UTC()}
			f.records[collection+"/"+key] = o
			observations = append(observations, o)
		}
		latest := float64(0)
		rows := map[string]map[string]any{}
		var current *usageEvent
		locked := float64(0)
		closeSession := func(end float64) {
			if current == nil || end <= current.At {
				return
			}
			capped := math.Min(end, current.At+14400)
			credited := capped
			if locked > 0 {
				credited = math.Min(credited, locked)
			}
			for cursor := current.At; cursor < capped; {
				at := time.Unix(0, int64(cursor*1e9)).In(time.Local)
				next := time.Date(at.Year(), at.Month(), at.Day()+1, 0, 0, 0, 0, time.Local)
				until := math.Min(capped, float64(next.Unix()))
				day := at.Format("2006-01-02") + "@" + time.Local.String()
				key := batch.Device + "/" + day + "/" + current.Bundle
				row := rows[key]
				if row == nil {
					row = map[string]any{"device_id": map[string]any{"$ref": map[string]string{"collection": "usage.device", "native_key": batch.Device}}, "day": day, "bundle_id": current.Bundle, "seconds_estimated": float64(0), "clipped_seconds": float64(0), "sessions": float64(0), "provenance": "biome.app_infocus", "decoder_version": "infocus-segb-1", "uncertainty": "focus estimates; remote batches may lag; settings_parity: unproven"}
					rows[key] = row
				}
				seconds := math.Max(0, math.Min(until, credited)-cursor)
				row["seconds_estimated"] = row["seconds_estimated"].(float64) + seconds
				row["clipped_seconds"] = row["clipped_seconds"].(float64) + until - cursor - seconds
				row["sessions"] = row["sessions"].(float64) + 1
				row["latest_event_at"] = time.Unix(0, int64(end*1e9)).UTC().Format(time.RFC3339Nano)
				cursor = until
			}
		}
		var previous usageEvent
		for i, event := range sorted {
			if event.At < 1.6e9 || event.At > 2.1e9 || event.Bundle == "" {
				continue
			}
			if i > 0 && event == previous {
				continue
			}
			previous = event
			latest = math.Max(latest, event.At)
			if batch.Platform == "mac" && (event.Bundle == "loginwindow" || event.Bundle == "com.apple.ScreenSaver.Engine" || event.Bundle == "bridge.power.sleep") {
				if event.Starting && current != nil && locked == 0 {
					locked = event.At
				}
				continue
			}
			if event.Starting {
				closeSession(event.At)
				copy := event
				current = &copy
				locked = 0
			} else if current != nil && current.Bundle == event.Bundle {
				closeSession(event.At)
				current = nil
				locked = 0
			}
		}
		if locked > 0 {
			closeSession(locked)
		}
		add("usage.device", batch.Device, map[string]any{"name": "Synthetic Device", "platform": batch.Platform, "build": "sim", "is_local": batch.Device == "local", "latest_event_at": time.Unix(0, int64(latest*1e9)).UTC().Format(time.RFC3339Nano), "provenance": "biome.app_infocus", "decoder_version": "infocus-segb-1"})
		for key, row := range rows {
			add("usage.app_day", key, row)
		}
		err := f.changedLocked(contracts.Usage)
		revision := f.durable.Revision[contracts.Usage]
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		if f.enqueue(r.Context(), contracts.Capture{Source: contracts.Usage, Observations: observations}, true, revision) != nil {
			http.Error(w, "capture", 500)
			return
		}
		json.NewEncoder(w).Encode(map[string]bool{"effect_done": true})
	})
}

func init() {
	sourceRoutes = append(sourceRoutes, (*fakeApple).usageControl)
}
