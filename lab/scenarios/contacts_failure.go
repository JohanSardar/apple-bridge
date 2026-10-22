package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func contactsVisibilityFailure(ctx context.Context, w *world.World, runID, tag, expected string, observers []*watcher) {
	details := map[string]any{"expected": expected}
	truth, err := w.Truth("observe", map[string]any{"source": "contacts", "tag": tag})
	details["truth_read_succeeded"] = err == nil
	details["truth_records"] = truth["records"]
	for i, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		records, err := contactRecords(ctx, w, endpoint)
		selected := []map[string]any{}
		for _, record := range records {
			var data map[string]any
			if json.Unmarshal(record.Data, &data) != nil || data["given_name"] != tag+" Testperson" {
				continue
			}
			selected = append(selected, map[string]any{"job_title": data["job_title"], "version": record.Version, "deleted": record.Deleted})
		}
		seen := []map[string]any{}
		observers[i].mu.Lock()
		for _, change := range observers[i].changes {
			var data map[string]any
			if json.Unmarshal(change.Change.Record.Data, &data) != nil || data["given_name"] != tag+" Testperson" {
				continue
			}
			seen = append(seen, map[string]any{"job_title": data["job_title"], "version": change.Change.Record.Version, "uptime_ns": change.UptimeNS})
		}
		invalid := observers[i].invalid
		observers[i].mu.Unlock()
		details[endpoint.Host] = map[string]any{"records": selected, "observer": seen, "read_succeeded": err == nil, "invalid_sse": invalid}
	}
	dir := filepath.Join("lab/evidence/private/contacts", runID)
	os.MkdirAll(dir, 0700)
	raw, _ := json.Marshal(details)
	os.WriteFile(filepath.Join(dir, "visibility-failure.json"), raw, 0600)
}
