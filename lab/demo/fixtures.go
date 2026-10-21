package demo

import (
	"context"
	"fmt"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func Fixtures(ctx context.Context, w *world.World) (map[string]string, error) {
	ids := map[string]string{}
	for _, collection := range []string{"messages.chat", "messages.handle", "messages.message"} {
		after := ""
		found := 0
		for {
			var page contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, IncludeAliases: true, After: after, Limit: 1000}, &page)
			if err != nil {
				return nil, err
			}
			for _, alias := range page.Aliases {
				if collection == "messages.message" && !strings.HasPrefix(alias.NativeKey, "demo-message-00-") {
					continue
				}
				if _, exists := ids[alias.NativeKey]; !exists {
					found++
				}
				ids[alias.NativeKey] = alias.RecordID
			}
			if page.Next == "" || collection == "messages.message" && found == 200 {
				break
			}
			after = page.Next
		}
	}
	if len(ids) != 300 {
		return nil, fmt.Errorf("demo fixtures incomplete: %d of 300", len(ids))
	}
	return ids, nil
}
