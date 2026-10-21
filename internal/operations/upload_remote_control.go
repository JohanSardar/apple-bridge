package operations

import (
	"context"
	"encoding/json"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (g *RemoteGateway) UploadControl(ctx context.Context, p trust.Principal, r contracts.UploadRequest) (contracts.UploadStatus, error) {
	var out contracts.UploadStatus
	if g.UploadExchange == nil || !g.Connected() {
		return out, contracts.ErrUnavailable
	}
	current, err := g.Authority.Principal(p.ID)
	if err != nil {
		return out, err
	}
	current.Grants = trust.EffectiveGrants(current.Grants, p.Grants)
	if current.IsAgent() && current.Level == trust.AgentRead {
		return out, contracts.ErrUnauthorized
	}
	if err = g.Authority.Authorize(current, "messages:write"); err != nil {
		return out, err
	}
	if r.Action == "begin" || r.Action == "prepare" || r.Action == "commit" {
		if !contracts.ValidDigest(r.Digest) || r.Action == "begin" && r.Size < 0 {
			return out, contracts.ErrInvalid
		}
	}
	raw, err := json.Marshal(r)
	if err != nil {
		return out, err
	}
	header, err := g.Authority.SignDelegation(current, r.ID, trust.Fingerprint(raw))
	if err != nil {
		return out, err
	}
	response, err := g.UploadExchange(ctx, header, raw)
	if err != nil {
		return out, err
	}
	return decodeUploadResponse(response, r.ID)
}
func decodeUploadResponse(response contracts.CommandResponse, id string) (contracts.UploadStatus, error) {
	var out contracts.UploadStatus
	if response.Error != nil {
		return out, response.Error
	}
	if json.Unmarshal(response.Result, &out) != nil || out.ID != id || out.Offset < 0 || out.Offset > contracts.MessageFileLimit || out.Size < 0 || out.Size > contracts.MessageFileLimit || out.Offset > out.Size || out.State != "uploading" && out.State != "ready" || out.ExpiresAt.IsZero() {
		return out, contracts.ErrCorrupt
	}
	return out, nil
}
