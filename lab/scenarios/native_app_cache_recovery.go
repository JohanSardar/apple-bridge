package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
)

func nativeCacheRecovers(ctx context.Context, cache, key string) bool {
	proof := filepath.Join(cache, key, "helper.json")
	bytes, err := os.ReadFile(proof)
	if err != nil {
		return false
	}
	var fields map[string]any
	if json.Unmarshal(bytes, &fields) != nil {
		return false
	}
	fields["bundle_sha256"] = "invalid"
	if os.WriteFile(proof, mustJSON(fields), 0600) != nil {
		return false
	}
	output, err := os.MkdirTemp("", "ab-native-cache-recovery-")
	if err != nil {
		return false
	}
	defer os.RemoveAll(output)
	app := filepath.Join(output, "AppleBridgeHelper.app")
	if exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).Run() != nil {
		return false
	}
	bytes, err = os.ReadFile(filepath.Join(output, "AppleBridgeHelper.cache.json"))
	var report struct {
		Hit       bool   `json:"hit"`
		Recovered bool   `json:"recovered_corrupt_entry"`
		Key       string `json:"key"`
	}
	if err != nil || json.Unmarshal(bytes, &report) != nil || report.Hit || !report.Recovered || report.Key != key {
		return false
	}
	return exec.CommandContext(ctx, "codesign", "--verify", "--strict", app).Run() == nil
}
