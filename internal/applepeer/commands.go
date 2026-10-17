package applepeer

import (
	"context"
	"sync"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type commandSession struct {
	wire    *Wire
	fence   uint64
	done    chan struct{}
	waiting map[string]chan contracts.IPCFrame
}
type Commands struct {
	mu      sync.Mutex
	session *commandSession
}

func (c *Commands) attach(w *Wire, fence uint64) *commandSession {
	c.mu.Lock()
	defer c.mu.Unlock()
	s := &commandSession{wire: w, fence: fence, done: make(chan struct{}), waiting: map[string]chan contracts.IPCFrame{}}
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
	if ch := s.waiting[f.CommandID]; ch != nil {
		select {
		case ch <- f:
		default:
		}
	}
}
func (c *Commands) Available() bool { c.mu.Lock(); defer c.mu.Unlock(); return c.session != nil }
func (c *Commands) Exchange(ctx context.Context, f contracts.IPCFrame) (contracts.IPCFrame, error) {
	c.mu.Lock()
	s := c.session
	if s == nil {
		c.mu.Unlock()
		return contracts.IPCFrame{}, contracts.ErrUnavailable
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
	if _, err := s.wire.Send(f); err != nil {
		return contracts.IPCFrame{}, contracts.ErrUnavailable
	}
	select {
	case answer := <-ch:
		return answer, nil
	case <-s.done:
		return contracts.IPCFrame{}, contracts.ErrUnavailable
	case <-ctx.Done():
		return contracts.IPCFrame{}, ctx.Err()
	}
}
