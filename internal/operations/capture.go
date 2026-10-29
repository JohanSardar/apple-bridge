package operations

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"time"
)

func (g *LocalGateway) saveCaptureBlob(r contracts.CommandRecord, f *contracts.IPCFrame) error {
	var e struct {
		Digest string `json:"digest"`
		Size   int64  `json:"size"`
		Handle string `json:"handle"`
	}
	if json.Unmarshal(f.Evidence, &e) != nil || e.Handle == "" || g.Uploads == nil {
		return contracts.ErrCorrupt
	}
	b := contracts.QueryBlob{Digest: e.Digest, Size: e.Size, Handle: e.Handle}
	ctx, cancel := context.WithTimeout(g.Context, 75*time.Second)
	defer cancel()
	data, err := g.Peer.QueryBlob(ctx, b)
	if err != nil {
		return err
	}
	_, pin, err := g.Uploads.Stage(ctx, data, blobs.Expect{Digest: b.Digest, Size: b.Size})
	if pin != "" {
		defer g.Uploads.Release(context.WithoutCancel(ctx), pin)
	}
	if err != nil {
		return err
	}
	id := contracts.NewID("qry")
	b.Handle = ""
	d, _ := contracts.Descriptor(r.Command.Op)
	if err = g.State.SaveQueryBlobs(ctx, r.Principal, id, d.Scope, []contracts.QueryBlob{b}); err != nil {
		return err
	}
	var fields map[string]any
	json.Unmarshal(f.Evidence, &fields)
	delete(fields, "handle")
	fields["blob_id"] = id
	f.Evidence, _ = json.Marshal(fields)
	return nil
}

func (g *RemoteGateway) retainCaptureGrant(ctx context.Context, p trust.Principal, r contracts.CommandRecord) error {
	if !contracts.CaptureOperation(r.Command.Op) || r.State != contracts.CommandObserved {
		return nil
	}
	var e struct {
		ID     string `json:"blob_id"`
		Digest string `json:"digest"`
		Size   int64  `json:"size"`
	}
	if json.Unmarshal(r.Evidence, &e) != nil || e.ID == "" || !contracts.CaptureEvidence(r.Evidence, r.RequestHash) {
		return contracts.ErrCorrupt
	}
	if grant, err := g.State.QueryBlobGrant(ctx, p, e.ID); err == nil && grant.AllowsBlob(e.Digest) {
		return nil
	}
	d, _ := contracts.Descriptor(r.Command.Op)
	return g.State.SaveQueryBlobs(ctx, p.ID, e.ID, d.Scope, []contracts.QueryBlob{{Digest: e.Digest, Size: e.Size}})
}
