package iphone

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type CaptureUpload struct {
	ID          string `json:"command_id"`
	Hash        string `json:"request_hash"`
	ContentType string `json:"content_type"`
	Data        []byte `json:"data"`
}

type CaptureReceipt struct {
	BlobID      string `json:"blob_id"`
	Digest      string `json:"digest"`
	Size        int64  `json:"size"`
	ContentType string `json:"content_type"`
}

func (d *Delivery) Capture(ctx context.Context, phone trust.Principal, payload []byte) (CaptureReceipt, error) {
	d.mu.Lock()
	defer d.mu.Unlock()
	var upload CaptureUpload
	var out CaptureReceipt
	if contracts.DecodeInput(payload, &upload) != nil || len(upload.Data) == 0 || len(upload.Data) > 12<<20 || d.Blobs == nil {
		return out, contracts.ErrInvalid
	}
	r, err := d.State.Command(ctx, upload.ID)
	if err != nil {
		return out, err
	}
	if !contracts.IPhoneCaptureCommand(r.Command.Op) || targetOf(r) != phone.ID || r.RequestHash != upload.Hash || r.State != contracts.CommandDispatched || !time.Now().Before(r.ExpiresAt) {
		return out, contracts.ErrUnauthorized
	}
	if d.Authorize == nil {
		return out, contracts.ErrUnauthorized
	}
	if err = d.Authorize(ctx, r); err != nil {
		return out, err
	}
	expected := "image/jpeg"
	if r.Command.Op == "iphone.capture.audio" {
		expected = "audio/wav"
	}
	if upload.ContentType != expected {
		return out, contracts.ErrInvalid
	}
	principal, err := d.Authority.Principal(r.Principal)
	if err != nil {
		return out, err
	}
	hash := sha256.Sum256(upload.Data)
	idHash := sha256.Sum256([]byte(r.Command.ID + "\x00" + r.RequestHash))
	out = CaptureReceipt{BlobID: "qry_" + hex.EncodeToString(idHash[:]), Digest: hex.EncodeToString(hash[:]), Size: int64(len(upload.Data)), ContentType: expected}
	if grant, grantErr := d.State.QueryBlobGrant(ctx, principal, out.BlobID); grantErr == nil {
		if !grant.AllowsBlob(out.Digest) {
			return CaptureReceipt{}, contracts.ErrInvalid
		}
		return out, nil
	}
	_, pin, err := d.Blobs.Stage(ctx, bytes.NewReader(upload.Data), blobs.Expect{Digest: out.Digest, Size: out.Size})
	if pin != "" {
		defer d.Blobs.Release(context.WithoutCancel(ctx), pin)
	}
	if err != nil {
		return CaptureReceipt{}, err
	}
	err = d.State.SaveQueryBlobs(ctx, r.Principal, out.BlobID, "iphone:write", []contracts.QueryBlob{{Digest: out.Digest, Size: out.Size}})
	return out, err
}
