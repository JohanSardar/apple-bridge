package simapple

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"sync"
	"time"
)

type querySession struct {
	ctx     context.Context
	fake    *fakeApple
	wire    *applepeer.Wire
	mu      sync.Mutex
	running map[string]context.CancelFunc
}

func newQuerySession(ctx context.Context, f *fakeApple, w *applepeer.Wire) *querySession {
	return &querySession{ctx: ctx, fake: f, wire: w, running: map[string]context.CancelFunc{}}
}
func (s *querySession) receive(f contracts.IPCFrame) bool {
	if f.T != "Query" && f.T != "QueryCancel" {
		return false
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	if f.T == "QueryCancel" {
		if cancel := s.running[f.ID]; cancel != nil {
			cancel()
		}
		return true
	}
	if s.running[f.ID] != nil {
		return true
	}
	s.fake.mu.Lock()
	s.fake.effects[f.ID] = 1
	s.fake.mu.Unlock()
	ctx, cancel := context.WithTimeout(s.ctx, time.Duration(f.DeadlineMS)*time.Millisecond)
	s.running[f.ID] = cancel
	go func() {
		defer func() { s.fake.mu.Lock(); s.fake.effects[f.ID] = 0; s.fake.mu.Unlock() }()
		defer cancel()
		defer func() { s.mu.Lock(); delete(s.running, f.ID); s.mu.Unlock() }()
		out := contracts.IPCFrame{T: "QueryResult", ID: f.ID, ResultJSON: f.Args, Blobs: []contracts.QueryBlob{}}
		var args struct {
			MS   int `json:"ms"`
			Size int `json:"size"`
		}
		decodeErr := contracts.DecodeInput(f.Args, &args)
		var fields map[string]json.RawMessage
		json.Unmarshal(f.Args, &fields)
		field := "ms"
		if f.Op == "test.blob" {
			field = "size"
		}
		if (f.Op == "test.sleep" || f.Op == "test.blob") && (decodeErr != nil || len(fields) != 1 || fields[field] == nil || args.MS < 0 || args.MS > 60000 || args.Size < 0 || args.Size > 64<<20) {
			_, _ = s.wire.Send(contracts.IPCFrame{T: "QueryError", ID: f.ID, Code: "invalid"})
			return
		}
		switch f.Op {
		case "test.echo":
		case "test.sleep":
			select {
			case <-ctx.Done():
				out.T = "QueryError"
				out.Code = "deadline"
			case <-time.After(time.Duration(args.MS) * time.Millisecond):
			}
		case "test.blob":
			if args.Size < 0 || args.Size > 64<<20 {
				out.T = "QueryError"
				out.Code = "invalid"
				break
			}
			data := make([]byte, args.Size)
			for i := range data {
				data[i] = byte(i % 251)
			}
			sum := sha256.Sum256(data)
			digest := hex.EncodeToString(sum[:])
			s.fake.mu.Lock()
			s.fake.blobs[digest] = data
			s.fake.mu.Unlock()
			out.ResultJSON = json.RawMessage(`{}`)
			out.Blobs = []contracts.QueryBlob{{Digest: digest, Size: int64(len(data)), Handle: digest}}
		case "helper.version":
			out.ResultJSON = json.RawMessage(`{"version":"sim","protocol":"ipc/1"}`)
		default:
			if code, result, handled := simulatePDFQuery(f.Op, f.Args); handled {
				out.ResultJSON = result
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				}
				break
			}
			if code, result, handled := simulateVisionQuery(f.Op, f.Args); handled {
				out.ResultJSON = result
				if code != "" {
					out.T = "QueryError"
					out.Code = code
				}
				break
			}
			out.T = "QueryError"
			out.Code = "unsupported"
		}
		if ctx.Err() == nil || out.T == "QueryError" {
			_, _ = s.wire.Send(out)
		}
	}()
	return true
}
