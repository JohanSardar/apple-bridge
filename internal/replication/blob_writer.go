package replication

import (
	"context"
	"github.com/coder/websocket"
)

type writeJob struct {
	ctx   context.Context
	kind  websocket.MessageType
	bytes []byte
	done  chan error
}
type writerKey struct{}
type lanes struct {
	live chan writeJob
	bulk chan writeJob
}

func priorityWriter(ctx context.Context, conn *websocket.Conn) context.Context {
	q := &lanes{live: make(chan writeJob, 16), bulk: make(chan writeJob, 1)}
	go func() {
		for {
			var job writeJob
			select {
			case <-ctx.Done():
				return
			case job = <-q.live:
			default:
				select {
				case <-ctx.Done():
					return
				case job = <-q.live:
				case job = <-q.bulk:
				}
			}
			if job.kind == websocket.MessageBinary {
				select {
				case live := <-q.live:
					live.done <- writeSocketFrame(live.ctx, conn, live.kind, live.bytes)
				default:
				}
			}
			job.done <- writeSocketFrame(job.ctx, conn, job.kind, job.bytes)
		}
	}()
	return context.WithValue(ctx, writerKey{}, q)
}
func writeFrame(ctx context.Context, conn *websocket.Conn, kind websocket.MessageType, b []byte, bulk bool) error {
	q, _ := ctx.Value(writerKey{}).(*lanes)
	if q == nil {
		return conn.Write(ctx, kind, b)
	}
	job := writeJob{ctx: ctx, kind: kind, bytes: b, done: make(chan error, 1)}
	lane := q.live
	if bulk {
		lane = q.bulk
	}
	select {
	case lane <- job:
	case <-ctx.Done():
		return ctx.Err()
	}
	select {
	case err := <-job.done:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}
