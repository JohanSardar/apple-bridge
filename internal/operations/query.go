package operations

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
	"time"
)

type QueryAudit struct {
	Operation     string `json:"operation"`
	ArgumentBytes int    `json:"argument_bytes"`
	ResultBytes   int    `json:"result_bytes"`
	BlobBytes     int64  `json:"blob_bytes"`
}
type queryWindow struct {
	At    time.Time
	Count int
}

func (g *LocalGateway) admitQuery(principal string) error {
	g.mu.Lock()
	defer g.mu.Unlock()
	if g.queryRates == nil {
		g.queryRates = map[string]queryWindow{}
	}
	now := time.Now()
	for id, w := range g.queryRates {
		if now.Sub(w.At) > time.Second {
			delete(g.queryRates, id)
		}
	}
	w := g.queryRates[principal]
	if w.Count == 0 {
		w.At = now
	}
	w.Count++
	g.queryRates[principal] = w
	if w.Count > 100 {
		return contracts.ErrCapacity
	}
	return nil
}
func queryContext(ctx context.Context, q *contracts.Query) (context.Context, context.CancelFunc, error) {
	if q.ID == "" {
		q.ID = contracts.NewID("qry")
	}
	if len(q.ID) <= 4 || !contracts.ValidCommandID(q.ID) || !strings.HasPrefix(q.ID, "qry_") || len(q.ID) > 128 || q.DeadlineMS < 0 || q.DeadlineMS > 60000 || !json.Valid(q.Args) {
		return ctx, func() {}, contracts.ErrInvalid
	}
	if q.Op == "helper.version" || q.Op == "mac.clipboard_read" {
		var args struct{}
		if !bytes.HasPrefix(bytes.TrimSpace(q.Args), []byte("{")) || contracts.DecodeInput(q.Args, &args) != nil {
			return ctx, func() {}, contracts.ErrInvalid
		}
	}
	if q.DeadlineMS == 0 {
		q.DeadlineMS = 1000
	}
	next, cancel := context.WithTimeout(ctx, time.Duration(q.DeadlineMS)*time.Millisecond)
	return next, cancel, nil
}
func (g *LocalGateway) Query(ctx context.Context, p trust.Principal, q contracts.Query) (out contracts.QueryResult, err error) {
	d, ok := contracts.Descriptor(q.Op)
	if !ok || d.Kind != contracts.QueryOp {
		return out, contracts.ErrUnsupported
	}
	if err = g.authorize(p, q.Op); err != nil {
		if errors.Is(err, contracts.ErrUnauthorized) {
			err = contracts.ErrForbidden
		}
		return
	}
	if err = g.admitQuery(p.ID); err != nil {
		return
	}
	ctx, cancel, err := queryContext(ctx, &q)
	if err != nil {
		return
	}
	defer cancel()
	defer func() {
		if errors.Is(err, context.DeadlineExceeded) {
			err = contracts.ErrDeadline
		}
	}()
	if g.Peer == nil {
		return out, contracts.ErrUnavailable
	}
	defer func() {
		if g.QueryAudit != nil {
			record := QueryAudit{Operation: q.Op, ArgumentBytes: len(q.Args), ResultBytes: len(out.ResultJSON)}
			for _, b := range out.Blobs {
				record.BlobBytes += b.Size
			}
			g.QueryAudit(record)
		}
		if g.State != nil && p.IsAgent() && ctx.Err() == nil {
			auditErr := g.State.RecordAgentActivity(context.WithoutCancel(ctx), state.AgentActivity{ID: p.ID + ":" + q.ID, Principal: p.ID, Operation: q.Op, Kind: "query", State: "finished", At: time.Now().UTC(), ArgumentBytes: len(q.Args)})
			if err == nil {
				err = auditErr
			}
		}
	}()
	if q.Op == "mail.body" {
		if g.State == nil {
			return out, contracts.ErrUnavailable
		}
		var args struct {
			MessageID string `json:"message_id"`
		}
		if contracts.DecodeInput(q.Args, &args) != nil || args.MessageID == "" {
			return out, contracts.ErrInvalid
		}
		records, readErr := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "mail.message", ID: args.MessageID})
		if readErr != nil || len(records.Records) != 1 {
			return out, contracts.ErrNotFound
		}
		native, nativeErr := g.State.NativeKey(ctx, "mail.message", args.MessageID)
		if nativeErr != nil {
			return out, nativeErr
		}
		q.Args, err = json.Marshal(map[string]string{"message_id": native})
		if err != nil {
			return out, err
		}
	}
	frame, err := g.Peer.Exchange(ctx, contracts.IPCFrame{T: "Query", ID: q.ID, Op: q.Op, Args: q.Args, DeadlineMS: q.DeadlineMS})
	if errors.Is(err, context.DeadlineExceeded) {
		return out, contracts.ErrDeadline
	}
	if err != nil {
		return out, err
	}
	if frame.T == "QueryError" {
		return out, &contracts.Error{Code: frame.Code, Message: "query failed"}
	}
	if frame.T != "QueryResult" || !json.Valid(frame.ResultJSON) || len(frame.ResultJSON) > contracts.MaxQueryJSON || len(frame.Blobs) > 16 {
		return out, contracts.ErrCorrupt
	}
	out = contracts.QueryResult{ID: q.ID, ResultJSON: frame.ResultJSON, Blobs: frame.Blobs}
	if out.Blobs == nil {
		out.Blobs = []contracts.QueryBlob{}
	}
	for i, b := range out.Blobs {
		if !contracts.ValidDigest(b.Digest) || b.Size < 0 || b.Size > 64<<20 || b.Handle == "" || g.Uploads == nil {
			return out, contracts.ErrCorrupt
		}
		data, readErr := g.Peer.QueryBlob(ctx, b)
		if readErr != nil {
			return out, readErr
		}
		_, pin, stageErr := g.Uploads.Stage(ctx, data, blobs.Expect{Digest: b.Digest, Size: b.Size})
		err = stageErr
		if pin != "" {
			defer g.Uploads.Release(context.WithoutCancel(ctx), pin)
		}
		if err != nil {
			return out, err
		}
		if err = ctx.Err(); err != nil {
			return out, err
		}
		out.Blobs[i].Handle = ""
	}
	if len(out.Blobs) > 0 {
		err = g.State.SaveQueryBlobs(ctx, p.ID, q.ID, d.Scope, out.Blobs)
	}
	return
}
func (g *RemoteGateway) Query(ctx context.Context, p trust.Principal, q contracts.Query) (out contracts.QueryResult, err error) {
	d, ok := contracts.Descriptor(q.Op)
	if !ok || d.Kind != contracts.QueryOp {
		return out, contracts.ErrUnsupported
	}
	if !trust.Subset([]contracts.Grant{d.Scope}, p.Grants) {
		return out, contracts.ErrForbidden
	}
	ctx, cancel, err := queryContext(ctx, &q)
	if err != nil {
		return
	}
	defer cancel()
	defer func() {
		if errors.Is(err, context.DeadlineExceeded) {
			err = contracts.ErrDeadline
		}
	}()
	err = g.call(ctx, p, forwarded{Method: "query", NativeQuery: &q}, &out)
	if errors.Is(ctx.Err(), context.DeadlineExceeded) {
		return out, contracts.ErrDeadline
	}
	if err == nil && len(out.Blobs) > 0 {
		err = g.State.SaveQueryBlobs(ctx, p.ID, out.ID, d.Scope, out.Blobs)
	}
	return
}
