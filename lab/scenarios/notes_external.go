package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type notesFirstVisibility struct {
	at        int64
	contentAt int64
	maxPollMS float64
	err       error
}

type notesVisibilityWatch struct {
	ready chan error
	first chan notesFirstVisibility
}

func watchNotesVisibility(ctx context.Context, endpoint world.Endpoint, id, title string) notesVisibilityWatch {
	watch := notesVisibilityWatch{ready: make(chan error, 1), first: make(chan notesFirstVisibility, 1)}
	go func() {
		armed := false
		var maxPollMS float64
		var firstAt int64
		for {
			started := world.UptimeNS()
			var result contracts.ReadResult
			_, err := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "notes.note", ID: id, Limit: 1}, &result)
			at := world.UptimeNS()
			if !armed {
				watch.ready <- err
				armed = true
			}
			if err != nil {
				watch.first <- notesFirstVisibility{err: err}
				return
			}
			maxPollMS = max(maxPollMS, float64(at-started)/float64(time.Millisecond))
			for _, record := range result.Records {
				var data struct {
					Title           string `json:"title"`
					PlainText       string `json:"plain_text"`
					RecentlyDeleted bool   `json:"recently_deleted"`
					BodyStatus      string `json:"body_status"`
				}
				if record.ID == id && !record.Deleted && json.Unmarshal(record.Data, &data) == nil && !data.RecentlyDeleted && data.Title == title {
					if firstAt == 0 {
						firstAt = at
					}
					if data.BodyStatus == "pending" || data.BodyStatus == "unavailable" || strings.TrimRight(data.PlainText, "\n") != title+"\nbody" {
						continue
					}
					watch.first <- notesFirstVisibility{at: firstAt, contentAt: at, maxPollMS: maxPollMS}
					return
				}
			}
			timer := time.NewTimer(10 * time.Millisecond)
			select {
			case <-ctx.Done():
				timer.Stop()
				watch.first <- notesFirstVisibility{err: ctx.Err()}
				return
			case <-timer.C:
			}
		}
	}()
	return watch
}

func measureNotesExternal(ctx context.Context, w *world.World, level, tag, id string, e *Evidence) error {
	endpoints := []world.Endpoint{w.Gateway, w.Replicas[0]}
	labels := []string{"gateway", "replica"}
	values := [2][]float64{}
	contents := [2][]float64{}
	maxPollMS := [2]float64{}
	var nativeID any
	samples := []notesVisibilitySample{}
	nativeHash := ""
	if aliases, err := gatewayAliases(w); err == nil {
		for key, recordID := range aliases {
			if recordID == id && strings.HasPrefix(key, "notes/notes.note/") {
				nativeHash = fmt.Sprintf("%x", sha256.Sum256([]byte(strings.TrimPrefix(key, "notes/notes.note/"))))
			}
		}
	}
	if level != "sim" {
		response, err := w.Truth("observe", map[string]any{"source": "notes", "tag": tag})
		if err != nil {
			return err
		}
		rows, _ := response["records"].([]any)
		if len(rows) != 1 {
			return fmt.Errorf("tagged note identity count %d", len(rows))
		}
		nativeID = rows[0].(map[string]any)["id"]
	}
	for i := 0; i < 20; i++ {
		if err := func() error {
			sampleCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
			defer cancel()
			title := fmt.Sprintf("%s edit-%d", tag, i)
			watches := []notesVisibilityWatch{}
			for _, endpoint := range endpoints {
				watches = append(watches, watchNotesVisibility(sampleCtx, endpoint, id, title))
			}
			for _, watch := range watches {
				if err := <-watch.ready; err != nil {
					return err
				}
			}
			effectNS := world.UptimeNS()
			if level == "sim" {
				data := map[string]any{"title": title, "plain_text": title + "\nbody\n", "locked": false, "editable": "plain", "folder_id": map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": "note-folder"}}}
				if err := w.ControlAction("/external-change", map[string]any{"key": "external-note", "collection": "notes.note", "data": data}); err != nil {
					return err
				}
			} else {
				response, err := w.Truth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "modify", "id": nativeID, "plain_text": title + "\nbody"})
				if err != nil {
					return err
				}
				stamp, ok := response["effect_uptime_ns"].(json.Number)
				if !ok {
					return fmt.Errorf("native save timestamp missing")
				}
				effectNS, err = stamp.Int64()
				if err != nil || effectNS <= 0 {
					return fmt.Errorf("native save timestamp invalid")
				}
			}
			sample := notesVisibilitySample{Save: effectNS}
			for index, watch := range watches {
				first := <-watch.first
				if first.err != nil {
					return first.err
				}
				if index == 0 {
					sample.Gateway = first.at
					sample.GatewayContent = first.contentAt
				} else {
					sample.Replica = first.at
					sample.ReplicaContent = first.contentAt
				}
				contents[index] = append(contents[index], float64(first.contentAt-effectNS)/float64(time.Millisecond))
				values[index] = append(values[index], float64(first.at-effectNS)/float64(time.Millisecond))
				maxPollMS[index] = max(maxPollMS[index], first.maxPollMS)
			}
			samples = append(samples, sample)
			return nil
		}(); err != nil {
			return fmt.Errorf("external sample %d: %w", i, err)
		}
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	notesStageLatencies(w, e, nativeHash, samples)
	e.Metrics["observer_poll_interval_ms"] = 10
	e.Metrics["observers_armed_before_save"] = true
	for index, label := range labels {
		latency := stats(values[index], 0)
		e.Latency["native_save_to_"+label+"_visible_ms"] = latency
		e.Latency["native_save_to_"+label+"_content_ms"] = stats(contents[index], 0)
		e.Metrics[label+"_max_poll_request_ms"] = maxPollMS[index]
		e.Assertions = append(e.Assertions, Assertion{Name: label + "_external_p95_under_1s", Passed: latency.P95 < 1000})
	}
	selected := 0
	if level == "replica" {
		selected = 1
	}
	e.Latency["native_save_to_copy_visible_ms"] = stats(values[selected], 0)
	return nil
}
