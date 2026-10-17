package replication

import (
	"context"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

type clockProbe struct {
	mu      sync.Mutex
	enabled bool
	id      string
	sent    time.Time
}

func (p *clockProbe) enable() { p.mu.Lock(); p.enabled = true; p.mu.Unlock() }

func (p *clockProbe) ping(ctx context.Context, conn *websocket.Conn) error {
	p.mu.Lock()
	if !p.enabled || p.id != "" && time.Since(p.sent) < 15*time.Second {
		p.mu.Unlock()
		return nil
	}
	p.id = contracts.NewID("clock")
	p.sent = time.Now()
	frame := contracts.FeedFrame{T: "ClockPing", ID: p.id, Clock: &contracts.ClockSample{T0: p.sent.UTC()}}
	p.mu.Unlock()
	_, err := send(ctx, conn, frame)
	return err
}

func (p *clockProbe) complete(frame contracts.FeedFrame, received time.Time) (contracts.ClockSample, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	if frame.Clock == nil || p.id == "" || frame.ID != p.id || !frame.Clock.T0.Equal(p.sent) {
		return contracts.ClockSample{}, false
	}
	p.id = ""
	sample := *frame.Clock
	sample.T0 = p.sent
	sample.T3 = received
	_, _, err := sample.Estimate()
	return sample, err == nil
}

func clockPong(ctx context.Context, conn *websocket.Conn, f contracts.FeedFrame, received time.Time) error {
	if f.ID == "" || f.Clock == nil || f.Clock.T0.IsZero() || !f.Clock.T1.IsZero() || !f.Clock.T2.IsZero() || !f.Clock.T3.IsZero() {
		return contracts.ErrInvalid
	}
	sample := *f.Clock
	sample.T1 = received.UTC()
	sample.T2 = time.Now().UTC()
	_, err := send(ctx, conn, contracts.FeedFrame{T: "ClockPong", ID: f.ID, Clock: &sample})
	return err
}
