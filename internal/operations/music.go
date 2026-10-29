package operations

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
)

func (g *LocalGateway) musicArgs(ctx context.Context, p trust.Principal, c contracts.Command) (contracts.MusicCommandArgs, *contracts.Error) {
	var args contracts.MusicCommandArgs
	if contracts.DecodeInput(c.Args, &args) != nil {
		return args, contracts.ErrInvalid
	}
	for _, target := range []struct {
		ID         *string
		Collection string
	}{{&args.ID, "music.library_item"}, {&args.PlaylistID, "music.playlist"}} {
		if *target.ID == "" {
			continue
		}
		visible, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: target.Collection, ID: *target.ID})
		if err != nil || len(visible.Records) != 1 {
			return args, contracts.ErrNotFound
		}
		key, err := g.State.NativeKey(ctx, target.Collection, *target.ID)
		if err != nil {
			return args, contracts.ErrNotFound
		}
		*target.ID = strings.ToUpper(key)
	}
	return args, nil
}
func musicPublicEvidence(raw json.RawMessage) json.RawMessage {
	var fields struct {
		Fingerprint string `json:"fingerprint,omitempty"`
		Verified    bool   `json:"verified"`
		Player      struct {
			State  string `json:"state"`
			Volume int    `json:"volume"`
		} `json:"player"`
	}
	json.Unmarshal(raw, &fields)
	result, _ := json.Marshal(fields)
	return result
}
