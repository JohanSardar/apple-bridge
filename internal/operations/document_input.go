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
		if args["file"] != nil && args["data_base64"] != nil {
			return nil, contracts.ErrInvalid
		}
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
		QueryID    string `json:"query_id"`
		Digest     string `json:"digest"`
		RecordID   string `json:"record_id"`
		Collection string `json:"collection"`
	}
	if contracts.DecodeInput(blob, &ref) != nil || !contracts.ValidDigest(ref.Digest) {
		return nil, contracts.ErrInvalid
	}
	if g.State == nil || g.Uploads == nil {
		return nil, contracts.ErrUnavailable
	}
	var grant trust.RecordGrant
	var err error
	if ref.QueryID != "" && ref.RecordID == "" && ref.Collection == "" {
		grant, err = g.State.QueryBlobGrant(ctx, p, ref.QueryID)
	} else if ref.QueryID == "" && ref.RecordID != "" && ref.Collection != "" {
		if _, known := contracts.Collections[ref.Collection]; !known {
			return nil, contracts.ErrInvalid
		}
		var records contracts.ReadResult
		records, err = g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: ref.Collection, ID: ref.RecordID})
		if err == nil {
			if len(records.Records) != 1 {
				return nil, contracts.ErrNotFound
			}
			grant = trust.GrantRecord(records.Records[0])
		}
	} else {
		return nil, contracts.ErrInvalid
	}
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

func (g *LocalGateway) stageDocumentInput(ctx context.Context, raw json.RawMessage) (json.RawMessage, func(), error) {
	var args map[string]json.RawMessage
	if json.Unmarshal(raw, &args) != nil {
		return nil, nil, contracts.ErrInvalid
	}
	var encoded string
	if args["data_base64"] == nil || json.Unmarshal(args["data_base64"], &encoded) != nil || len(encoded) <= 128<<10 {
		return raw, func() {}, nil
	}
	if args["file"] != nil || len(encoded) > base64.StdEncoding.EncodedLen(8<<20) {
		return nil, nil, contracts.ErrInvalid
	}
	data, err := base64.StdEncoding.DecodeString(encoded)
	if err != nil {
		return nil, nil, contracts.ErrInvalid
	}
	if g.Uploads == nil {
		return nil, nil, contracts.ErrUnavailable
	}
	path, cleanup, err := g.Uploads.DocumentInput(ctx, data)
	if err != nil {
		return nil, nil, err
	}
	delete(args, "data_base64")
	args["file"], err = json.Marshal(path)
	if err != nil {
		cleanup()
		return nil, nil, err
	}
	out, err := json.Marshal(args)
	if err != nil {
		cleanup()
		return nil, nil, err
	}
	return out, cleanup, nil
}
