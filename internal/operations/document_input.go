package operations

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (g *LocalGateway) documentInput(ctx context.Context, p trust.Principal, raw json.RawMessage) (json.RawMessage, error) {
	var args map[string]json.RawMessage
	if json.Unmarshal(raw, &args) != nil || args == nil {
		return nil, contracts.ErrInvalid
	}
	blob, exists := args["blob"]
	if !exists {
		if args["file"] != nil {
			var allow bool
			if json.Unmarshal(args["allow_file"], &allow) != nil || !allow {
				return nil, contracts.ErrInvalid
			}
			delete(args, "allow_file")
			return json.Marshal(args)
		}
		return raw, nil
	}
	if args["file"] != nil || args["data_base64"] != nil || args["allow_file"] != nil {
		return nil, contracts.ErrInvalid
	}
	var ref struct {
		QueryID string `json:"query_id"`
		Digest  string `json:"digest"`
	}
	if contracts.DecodeInput(blob, &ref) != nil || ref.QueryID == "" || !contracts.ValidDigest(ref.Digest) {
		return nil, contracts.ErrInvalid
	}
	if g.State == nil || g.Uploads == nil {
		return nil, contracts.ErrUnavailable
	}
	grant, err := g.State.QueryBlobGrant(ctx, p, ref.QueryID)
	if err != nil {
		return nil, err
	}
	file, err := g.Uploads.OpenAuthorized(ctx, grant, ref.Digest)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	data, err := io.ReadAll(io.LimitReader(file, (8<<20)+1))
	if err != nil {
		return nil, err
	}
	if len(data) == 0 || len(data) > 8<<20 {
		return nil, contracts.ErrInvalid
	}
	delete(args, "blob")
	args["data_base64"], err = json.Marshal(base64.StdEncoding.EncodeToString(data))
	if err != nil {
		return nil, err
	}
	return json.Marshal(args)
}
