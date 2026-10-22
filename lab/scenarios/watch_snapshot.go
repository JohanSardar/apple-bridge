package scenarios

import (
	"bytes"
	"context"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func (w *watcher) resetSnapshot(ctx context.Context, endpoint world.Endpoint) error {
	records := map[string]contracts.Record{}
	after := ""
	var cursor contracts.Cursor
	for {
		var page contracts.ReadResult
		if _, err := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Limit: 128, After: after, IncludeDeleted: true}, &page); err != nil {
			return err
		}
		if after == "" {
			cursor = page.Cursor
		} else if page.Cursor != cursor {
			return contracts.ErrReset
		}
		for _, record := range page.Records {
			records[key(record)] = record
		}
		if page.Next == "" {
			break
		}
		after = page.Next
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	if cursor == w.last {
		w.invalid = true
		return contracts.ErrInvalid
	}
	at := time.Now()
	uptime := world.UptimeNS()
	for id, record := range records {
		if previous, ok := w.seen[id]; ok && previous.Change.Record.Version == record.Version && bytes.Equal(previous.Change.Record.Data, record.Data) {
			continue
		}
		if w.filtered && record.Source != contracts.System {
			w.invalid = true
		}
		op := "upsert"
		if record.Deleted {
			op = "delete"
		}
		w.seen[id] = Seen{Change: contracts.Change{Sequence: cursor.Sequence, Op: op, Record: record}, At: at, UptimeNS: uptime}
	}
	w.snapshotResets++
	w.last = cursor
	w.initialSequence = cursor.Sequence
	return nil
}
