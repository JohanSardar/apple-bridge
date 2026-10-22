package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayNativeReady(ctx context.Context, w *world.World, sources ...string) error {
	source := "reminders"
	if len(sources) > 0 {
		source = sources[0]
	}
	return w.Await(30*time.Second, func() bool {
		var result contracts.ReadResult
		if _, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", map[string]any{}, &result); err != nil {
			return false
		}
		for _, record := range result.Records {
			var data struct {
				Source string `json:"source"`
				State  string `json:"state"`
			}
			if json.Unmarshal(record.Data, &data) == nil && data.Source == source && data.State == "ready" {
				_, err := os.Stat(filepath.Join(w.Gateway.Home, "helper", "lab-committed-"+source))
				return err == nil
			}
		}
		return false
	})
}
