package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type contactSeen struct {
	ID, Job string
	Deleted bool
}
type contactWatch struct {
	mu   sync.Mutex
	seen map[contactSeen]int64
	body io.ReadCloser
	done chan struct{}
}

func watchContacts(ctx context.Context, in input, tag string) (*contactWatch, error) {
	var head contracts.ReadResult
	if err := request(ctx, in, "records.query", contracts.ReadQuery{Limit: 1}, &head); err != nil {
		return nil, err
	}
	req, err := http.NewRequestWithContext(ctx, "GET", in.URL+"/v1/changes?after="+head.Cursor.EventID(), nil)
	if err != nil {
		return nil, err
	}
	req.Host = in.Host
	req.Header.Set("Authorization", "Bearer "+in.Token)
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	if response.StatusCode != 200 {
		response.Body.Close()
		return nil, fmt.Errorf("contacts SSE status %d", response.StatusCode)
	}
	w := &contactWatch{seen: map[contactSeen]int64{}, body: response.Body, done: make(chan struct{})}
	go func() {
		defer close(w.done)
		scanner := bufio.NewScanner(response.Body)
		scanner.Buffer(make([]byte, 4096), contracts.MaxFrame*2)
		for scanner.Scan() {
			line := scanner.Text()
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			at := time.Now().UnixNano()
			var batch contracts.Batch
			if json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &batch) != nil {
				return
			}
			for _, change := range batch.Changes {
				if change.Record.Collection != "contacts.contact" {
					continue
				}
				var data struct {
					Given        string `json:"given_name"`
					Job          string `json:"job_title"`
					Organization string `json:"organization"`
				}
				if json.Unmarshal(change.Record.Data, &data) != nil || !strings.HasPrefix(data.Given, tag+" ") || data.Organization != "Bridge Test" {
					continue
				}
				key := contactSeen{ID: change.Record.ID, Job: data.Job, Deleted: change.Record.Deleted}
				w.mu.Lock()
				if w.seen[key] == 0 {
					w.seen[key] = at
				}
				w.mu.Unlock()
			}
		}
	}()
	return w, nil
}
func (w *contactWatch) close() { w.body.Close(); <-w.done }
func (w *contactWatch) observed(id, job string, deleted bool) int64 {
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.seen[contactSeen{ID: id, Job: job, Deleted: deleted}]
}
