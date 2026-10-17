package scenarios

import (
	"bufio"
	"context"
	"encoding/json"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func openSSE(ctx context.Context, e world.Endpoint, id string) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", e.URL+"/v1/changes", nil)
	if err != nil {
		return nil, err
	}
	req.Host = e.Host
	req.Header.Set("Authorization", "Bearer "+e.Token)
	req.Header.Set("Last-Event-ID", id)
	return http.DefaultClient.Do(req)
}
func sseResume(ctx context.Context, w *world.World, e world.Endpoint) bool {
	cursor, err := w.Cursor(e)
	if err != nil {
		return false
	}
	for _, stale := range []contracts.Cursor{{Epoch: "prior-epoch", SubscriptionRevision: cursor.SubscriptionRevision, Sequence: cursor.Sequence}, {Epoch: cursor.Epoch, SubscriptionRevision: cursor.SubscriptionRevision + 1, Sequence: cursor.Sequence}} {
		resp, err := openSSE(ctx, e, stale.EventID())
		if err != nil {
			return false
		}
		resp.Body.Close()
		if resp.StatusCode != 409 {
			return false
		}
	}
	bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	resp, err := openSSE(bounded, e, cursor.EventID())
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return false
	}
	title := contracts.NewID("sse")
	if w.ControlAction("/external-change", map[string]string{"key": title}) != nil {
		return false
	}
	scanner := bufio.NewScanner(resp.Body)
	scanner.Buffer(make([]byte, 4096), contracts.MaxFrame*2)
	var id contracts.Cursor
	for scanner.Scan() {
		line := scanner.Text()
		if strings.HasPrefix(line, "id: ") {
			id, err = contracts.ParseEventID(strings.TrimPrefix(line, "id: "))
			if err != nil {
				return false
			}
		}
		if strings.HasPrefix(line, "data: ") {
			var batch contracts.Batch
			if json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &batch) != nil {
				return false
			}
			expected := contracts.Cursor{Epoch: batch.Epoch, SubscriptionRevision: batch.SubscriptionRevision, Sequence: batch.Through}
			if id != expected || batch.After != cursor.Sequence {
				return false
			}
			cursor = expected
			for _, change := range batch.Changes {
				if recordTitle(change.Record) == "[ab-test] "+title {
					return true
				}
			}
		}
	}
	return false
}
func sseIdle(ctx context.Context, w *world.World, e world.Endpoint) bool {
	bounded, cancel := context.WithTimeout(ctx, 23*time.Second)
	defer cancel()
	resp, err := openSSE(bounded, e, "")
	if err != nil {
		return false
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return false
	}
	if w.ControlAction("/external-change", map[string]string{"key": contracts.NewID("idle")}) != nil {
		return false
	}
	start := time.Now()
	scanner := bufio.NewScanner(resp.Body)
	heartbeats := 0
	for scanner.Scan() {
		if scanner.Text() == ": heartbeat" {
			heartbeats++
			if heartbeats >= 3 && time.Since(start) >= 14*time.Second {
				return true
			}
		}
	}
	return false
}
