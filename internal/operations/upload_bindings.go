package operations

import (
	"context"
	"strconv"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type uploadBinding struct {
	Session     string
	Fingerprint string
	Principal   trust.Principal
	Request     contracts.UploadRequest
	Expires     time.Time
}

func uploadBindingKey(session, digest string, offset int64) string {
	return session + ":" + digest + ":" + strconv.FormatInt(offset, 10)
}
func (g *LocalGateway) prepareUpload(ctx context.Context, session, fingerprint string, p trust.Principal, r contracts.UploadRequest) error {
	if !contracts.ValidCommandID(r.ChunkID) {
		return contracts.ErrInvalid
	}
	g.uploadMu.Lock()
	defer g.uploadMu.Unlock()
	if err := ctx.Err(); err != nil {
		return err
	}
	if g.uploadPending == nil {
		g.uploadPending = map[string]uploadBinding{}
	}
	count := 0
	for key, binding := range g.uploadPending {
		if !time.Now().Before(binding.Expires) {
			delete(g.uploadPending, key)
			continue
		}
		if binding.Fingerprint == fingerprint {
			count++
		}
	}
	key := uploadBindingKey(session, r.Digest, r.Offset)
	if old, ok := g.uploadPending[key]; ok {
		if old.Principal.ID != p.ID || old.Request.ID != r.ID {
			return contracts.ErrConflict
		}
	} else if count >= 16 {
		return contracts.ErrUnavailable
	}
	g.uploadPending[key] = uploadBinding{Session: session, Fingerprint: fingerprint, Principal: p, Request: r, Expires: time.Now().Add(30 * time.Second)}
	return nil
}
func (g *LocalGateway) UploadClosed(session string) {
	g.uploadMu.Lock()
	defer g.uploadMu.Unlock()
	for key, binding := range g.uploadPending {
		if binding.Session == session {
			delete(g.uploadPending, key)
		}
	}
}
