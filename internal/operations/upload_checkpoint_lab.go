//go:build lab

package operations

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
)

func uploadForwardCheckpoint(ctx context.Context, id string, offset, size int64) error {
	directory := filepath.Join(os.Getenv("APPLE_BRIDGE_HOME"), "upload-checkpoints", id)
	if os.Rename(filepath.Join(directory, "armed"), filepath.Join(directory, "held")) != nil {
		return nil
	}
	raw, _ := json.Marshal(map[string]any{"upload_id": id, "state": "paused", "offset": offset, "size": size})
	if err := os.WriteFile(filepath.Join(directory, "paused.tmp"), raw, 0600); err != nil {
		return err
	}
	if err := os.Rename(filepath.Join(directory, "paused.tmp"), filepath.Join(directory, "paused")); err != nil {
		return err
	}
	<-ctx.Done()
	if err := os.WriteFile(filepath.Join(directory, "canceled"), []byte("true"), 0600); err != nil {
		return err
	}
	return ctx.Err()
}
