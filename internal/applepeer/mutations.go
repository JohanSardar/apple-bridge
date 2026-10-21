package applepeer

import (
	"context"
	"sync"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type mutation struct {
	frame contracts.IPCFrame
	bytes int
}

type mutations struct {
	mu                                     sync.Mutex
	pending                                []mutation
	captureBytes, stateBytes, commandBytes int
	changed                                chan struct{}
	done                                   chan struct{}
	cancel                                 context.CancelFunc
}

func startMutations(ctx context.Context, wire *Wire, apply func(contracts.IPCFrame, int) (contracts.IPCFrame, error)) *mutations {
	ctx, cancel := context.WithCancel(ctx)
	q := &mutations{changed: make(chan struct{}, 1), done: make(chan struct{}), cancel: cancel}
	go func() {
		defer close(q.done)
		for {
			q.mu.Lock()
			if ctx.Err() != nil {
				q.mu.Unlock()
				return
			}
			if len(q.pending) == 0 {
				q.mu.Unlock()
				select {
				case <-ctx.Done():
					return
				case <-q.changed:
				}
				continue
			}
			item := q.pending[0]
			q.pending[0] = mutation{}
			q.pending = q.pending[1:]
			q.mu.Unlock()
			reply, err := apply(item.frame, item.bytes)
			q.mu.Lock()
			switch item.frame.T {
			case "Capture":
				q.captureBytes -= item.bytes
			case "SourceState":
				q.stateBytes -= item.bytes
			default:
				q.commandBytes -= item.bytes
			}
			q.mu.Unlock()
			if err == nil && reply.T != "" {
				_, err = wire.Send(reply)
			}
			if err != nil {
				wire.Conn.Close()
				return
			}
		}
	}()
	return q
}

func (q *mutations) enqueue(frame contracts.IPCFrame, size int) bool {
	q.mu.Lock()
	defer q.mu.Unlock()
	if frame.T == "Capture" {
		if frame.Capture == nil || size > contracts.ByteCredit-q.captureBytes {
			return false
		}
		q.captureBytes += size
	} else if frame.T == "SourceState" {
		if size > contracts.MaxFrame+4-q.stateBytes {
			return false
		}
		q.stateBytes += size
	} else {
		if size > contracts.MaxFrame+4-q.commandBytes {
			return false
		}
		q.commandBytes += size
	}
	q.pending = append(q.pending, mutation{frame: frame, bytes: size})
	select {
	case q.changed <- struct{}{}:
	default:
	}
	return true
}

func (q *mutations) stop(wire *Wire) {
	q.cancel()
	wire.Conn.Close()
	<-q.done
}

func commitMutation(ctx, session context.Context, cfg Config, wire *Wire, frame contracts.IPCFrame, size int) (contracts.IPCFrame, error) {
	if frame.T == "SourceState" {
		return contracts.IPCFrame{}, cfg.SourceState(ctx, frame.Source, frame.State, frame.Coverage[frame.Source], frame.Capabilities[frame.Source], frame.Reason)
	}
	capture := *frame.Capture
	degraded, err := cfg.CommitCapture(ctx, capture)
	if err == contracts.ErrReset {
		return contracts.IPCFrame{T: "Reset", Source: capture.Source}, nil
	}
	if err != nil {
		cfg.SourceState(ctx, capture.Source, "degraded", contracts.Coverage{Fields: "partial", Missing: []string{"capture_failed"}}, nil, "capture_failed")
		return contracts.IPCFrame{}, err
	}
	if cfg.BlobCapture != nil {
		cfg.BlobCapture(session, capture, wire)
	}
	if degraded {
		cfg.SourceState(ctx, capture.Source, "degraded", contracts.Coverage{Fields: "partial", Missing: []string{"incomplete_scan_absence_ignored"}}, nil, "incomplete_scan")
	}
	return contracts.IPCFrame{T: "Committed", Source: capture.Source, Checkpoint: capture.Checkpoint, Credit: size}, nil
}
