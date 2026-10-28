package applepeer

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type commandSession struct {
	wire         *Wire
	fence        uint64
	done         chan struct{}
	waiting      map[string]chan contracts.IPCFrame
	receipts     map[string]captureReceipt
	receiptOrder []string
}
type captureReceipt struct {
	cursor       contracts.Cursor
	observations map[string]bool
}

type Commands struct {
	captureChanges map[contracts.Source]chan struct{}
	mu             sync.Mutex
	session        *commandSession
}

func (c *Commands) attach(w *Wire, fence uint64) *commandSession {
	c.mu.Lock()
	defer c.mu.Unlock()
	s := &commandSession{wire: w, fence: fence, done: make(chan struct{}), waiting: map[string]chan contracts.IPCFrame{}, receipts: map[string]captureReceipt{}}
	if c.session != nil {
		close(c.session.done)
	}
	c.session = s
	return s
}
func (c *Commands) detach(s *commandSession) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.session == s {
		close(s.done)
		c.session = nil
	}
}
func (c *Commands) receive(s *commandSession, f contracts.IPCFrame) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.session != s {
		return
	}
	if f.ID != "" {
		f.CommandID = f.ID
	}
	if ch := s.waiting[f.CommandID]; ch != nil {
		select {
		case ch <- f:
		default:
		}
	}
}
func (c *Commands) Available() bool { c.mu.Lock(); defer c.mu.Unlock(); return c.session != nil }
func (c *Commands) Exchange(ctx context.Context, f contracts.IPCFrame) (contracts.IPCFrame, error) {
	if f.ID != "" {
		f.CommandID = f.ID
	}
	c.mu.Lock()
	s := c.session
	if s == nil {
		c.mu.Unlock()
		return contracts.IPCFrame{}, contracts.ErrUnavailable
	}
	if len(s.waiting) >= 64 {
		c.mu.Unlock()
		return contracts.IPCFrame{}, contracts.ErrCapacity
	}
	if s.waiting[f.CommandID] != nil {
		c.mu.Unlock()
		return contracts.IPCFrame{}, contracts.ErrConflict
	}
	ch := make(chan contracts.IPCFrame, 1)
	s.waiting[f.CommandID] = ch
	if f.T == "Dispatch" {
		f.OwnerFence = s.fence
	}
	c.mu.Unlock()
	defer func() { c.mu.Lock(); delete(s.waiting, f.CommandID); c.mu.Unlock() }()
	var sendErr error
	if f.T == "Query" {
		_, sendErr = s.wire.SendContext(ctx, f)
	} else {
		_, sendErr = s.wire.Send(f)
	}
	if sendErr != nil {
		if f.T == "Query" {
			s.wire.Conn.Close()
			c.detach(s)
			if ctx.Err() != nil {
				return contracts.IPCFrame{}, ctx.Err()
			}
			if errors.Is(sendErr, context.DeadlineExceeded) {
				return contracts.IPCFrame{}, sendErr
			}
		}
		return contracts.IPCFrame{}, contracts.ErrUnavailable
	}
	select {
	case answer := <-ch:
		return answer, nil
	case <-s.done:
		return contracts.IPCFrame{}, contracts.ErrUnavailable
	case <-ctx.Done():
		if f.T == "Query" {
			cancelctx, stop := context.WithTimeout(context.Background(), 50*time.Millisecond)
			_, err := s.wire.SendContext(cancelctx, contracts.IPCFrame{T: "QueryCancel", ID: f.ID})
			stop()
			if err != nil {
				s.wire.Conn.Close()
				c.detach(s)
			}
		}
		return contracts.IPCFrame{}, ctx.Err()
	}
}

func (c *Commands) RecordCapture(capture contracts.Capture, cursor contracts.Cursor) {
	c.mu.Lock()
	defer c.mu.Unlock()
	s := c.session
	if s == nil {
		return
	}
	key := string(capture.Source) + "/" + capture.Checkpoint
	observations := map[string]bool{}
	for _, o := range capture.Observations {
		if o.Kind == "upsert" || o.Kind == "absent" {
			observations[o.Collection+"/"+o.NativeKey] = true
		}
	}
	if previous, ok := s.receipts[key]; ok {
		for observation := range previous.observations {
			observations[observation] = true
		}
	} else {
		s.receiptOrder = append(s.receiptOrder, key)
	}
	s.receipts[key] = captureReceipt{cursor: cursor, observations: observations}
	if len(s.receiptOrder) > 4096 {
		delete(s.receipts, s.receiptOrder[0])
		s.receiptOrder = s.receiptOrder[1:]
	}
	if changed := c.captureChanges[capture.Source]; changed != nil {
		close(changed)
		c.captureChanges[capture.Source] = make(chan struct{})
	}
}
func (c *Commands) CapturedSequence(source contracts.Source, checkpoint, collection, nativeKey string) (contracts.Cursor, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.session != nil {
		receipt, ok := c.session.receipts[string(source)+"/"+checkpoint]
		if ok && receipt.observations[collection+"/"+nativeKey] {
			return receipt.cursor, nil
		}
	}
	return contracts.Cursor{}, contracts.ErrNotFound
}

func (c *Commands) CaptureChanges(source contracts.Source) <-chan struct{} {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.captureChanges == nil {
		c.captureChanges = map[contracts.Source]chan struct{}{}
	}
	if c.captureChanges[source] == nil {
		c.captureChanges[source] = make(chan struct{})
	}
	return c.captureChanges[source]
}
