package replication

import (
	"context"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type feedReceipt struct {
	bytes  int
	cursor *contracts.Cursor
	done   chan struct{}
}

type feedWindow struct {
	mu        sync.Mutex
	limit     int
	available int
	pending   []*feedReceipt
	changed   chan struct{}
	timer     *time.Timer
}

func newFeedWindow(limit int) *feedWindow {
	return &feedWindow{limit: limit, available: limit, changed: make(chan struct{}, 1)}
}

func (w *feedWindow) reserve(ctx context.Context, cancel context.CancelFunc, size int, expected *contracts.Cursor) (*feedReceipt, error) {
	if size > w.limit {
		return nil, contracts.ErrCapacity
	}
	for {
		if err := ctx.Err(); err != nil {
			return nil, err
		}
		w.mu.Lock()
		if size <= w.available {
			receipt := &feedReceipt{bytes: size, done: make(chan struct{})}
			if expected != nil {
				cursor := *expected
				receipt.cursor = &cursor
			}
			if len(w.pending) == 0 {
				w.timer = time.AfterFunc(30*time.Second, cancel)
			}
			w.pending = append(w.pending, receipt)
			w.available -= size
			w.mu.Unlock()
			return receipt, nil
		}
		w.mu.Unlock()
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-w.changed:
		}
	}
}

func (w *feedWindow) acknowledge(ack contracts.FeedFrame) error {
	w.mu.Lock()
	defer w.mu.Unlock()
	if len(w.pending) == 0 {
		return contracts.ErrInvalid
	}
	receipt := w.pending[0]
	if ack.Credits != receipt.bytes || receipt.cursor != nil && (ack.Cursor == nil || *ack.Cursor != *receipt.cursor) {
		return contracts.ErrInvalid
	}
	w.pending[0] = nil
	w.pending = w.pending[1:]
	if len(w.pending) == 0 {
		w.timer.Stop()
	} else {
		w.timer.Reset(30 * time.Second)
	}
	w.available += receipt.bytes
	close(receipt.done)
	select {
	case w.changed <- struct{}{}:
	default:
	}
	return nil
}

func (w *feedWindow) close() {
	w.mu.Lock()
	defer w.mu.Unlock()
	if w.timer != nil {
		w.timer.Stop()
	}
}
