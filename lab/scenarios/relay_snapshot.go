package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayGatewaySnapshot(ctx context.Context, endpoint world.Endpoint) (relayQuery, error) {
	records := []contracts.Record{}
	after := ""
	var cursor contracts.Cursor
	for {
		var page contracts.ReadResult
		if _, err := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder", Limit: 200, After: after}, &page); err != nil {
			return relayQuery{}, err
		}
		if after == "" {
			cursor = page.Cursor
		} else if cursor != page.Cursor {
			return relayQuery{}, fmt.Errorf("snapshot changed during pagination")
		}
		records = append(records, page.Records...)
		if page.Next == "" {
			break
		}
		after = page.Next
	}
	raw, _ := json.Marshal(records)
	digest := sha256.Sum256(raw)
	return relayQuery{Count: len(records), Digest: hex.EncodeToString(digest[:]), Cursor: cursor}, nil
}
