package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"

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
		records = append(records, page.Records...)
		cursor = page.Cursor
		if page.Next == "" {
			break
		}
		after = page.Next
	}
	raw, _ := json.Marshal(records)
	digest := sha256.Sum256(raw)
	return relayQuery{Count: len(records), Digest: hex.EncodeToString(digest[:]), Cursor: cursor}, nil
}
