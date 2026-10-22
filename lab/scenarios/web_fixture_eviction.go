package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

type snapshotEvictionResult struct {
	Assertions map[string]bool `json:"assertions"`
	DiskBytes  int64           `json:"disk_bytes"`
	CacheBytes int64           `json:"cache_bytes"`
	LimitBytes int64           `json:"limit_bytes"`
}

func snapshotEvictionProof(ctx context.Context, directory string, prepare func(*world.World) (json.RawMessage, error)) (snapshotEvictionResult, error) {
	result := snapshotEvictionResult{Assertions: map[string]bool{}}
	stale := filepath.Join(directory, "building-stale-proof")
	fresh := filepath.Join(directory, "building-fresh-proof")
	if err := os.MkdirAll(stale, 0700); err != nil {
		return result, err
	}
	if err := os.MkdirAll(fresh, 0700); err != nil {
		return result, err
	}
	old := time.Now().Add(-2 * time.Minute)
	if err := os.Chtimes(stale, old, old); err != nil {
		return result, err
	}
	inspect := func(key, kind string, limit int64) (world.FixtureMetrics, error) {
		w, _, metrics, err := world.NewFixture(ctx, world.FixtureOptions{Directory: directory, Key: key, Kind: kind, CacheLimitBytes: limit, BuildTimeout: time.Minute}, prepare)
		if err == nil {
			w.Close()
		}
		return metrics, err
	}
	first, err := inspect("one", "same", 0)
	if err != nil {
		return result, err
	}
	_, staleErr := os.Stat(stale)
	_, freshErr := os.Stat(fresh)
	result.Assertions["stale_staging_reaped_fresh_staging_preserved"] = os.IsNotExist(staleErr) && freshErr == nil
	second, err := inspect("two", "same", 0)
	if err != nil {
		return result, err
	}
	hit, err := inspect("one", "same", 0)
	if err != nil {
		return result, err
	}
	var manifest struct {
		LastUsed time.Time `json:"last_used"`
		BuiltAt  time.Time `json:"built_at"`
	}
	raw, err := os.ReadFile(filepath.Join(directory, first.Key, "manifest.json"))
	if err != nil {
		return result, err
	}
	if err = json.Unmarshal(raw, &manifest); err != nil {
		return result, err
	}
	result.Assertions["cache_hit_updates_last_used"] = hit.Hit && manifest.LastUsed.After(manifest.BuiltAt)
	third, err := inspect("three", "same", 0)
	if err != nil {
		return result, err
	}
	_, firstErr := os.Stat(filepath.Join(directory, first.Key))
	_, secondErr := os.Stat(filepath.Join(directory, second.Key))
	entries, err := os.ReadDir(directory)
	if err != nil {
		return result, err
	}
	count := 0
	for _, entry := range entries {
		if entry.IsDir() && len(entry.Name()) == 64 {
			count++
		}
	}
	result.Assertions["three_keys_keep_two_most_recently_used_per_kind"] = count == 2 && firstErr == nil && os.IsNotExist(secondErr)
	limit := third.EntryBytes * 2
	for i := 0; i < 3; i++ {
		metrics, err := inspect(fmt.Sprintf("cap-%d", i), fmt.Sprintf("kind-%d", i), limit)
		if err != nil {
			return result, err
		}
		result.CacheBytes = metrics.CacheBytes
	}
	entries, err = os.ReadDir(directory)
	if err != nil {
		return result, err
	}
	for _, entry := range entries {
		if !entry.IsDir() || len(entry.Name()) != 64 {
			continue
		}
		output, err := exec.CommandContext(ctx, "du", "-sk", filepath.Join(directory, entry.Name())).Output()
		if err != nil {
			return result, err
		}
		fields := strings.Fields(string(output))
		if len(fields) == 0 {
			return result, fmt.Errorf("missing disk usage")
		}
		kib, err := strconv.ParseInt(fields[0], 10, 64)
		if err != nil {
			return result, err
		}
		result.DiskBytes += kib * 1024
	}
	result.LimitBytes = limit
	result.Assertions["total_cache_byte_cap_evicts_across_fixture_kinds"] = result.CacheBytes > 0 && result.CacheBytes <= limit && result.DiskBytes > 0 && result.DiskBytes <= limit

	for _, name := range []string{"APPLE_BRIDGE_FIXTURE_FREE_KB", "APPLE_BRIDGE_MIN_FREE_KB", "APPLE_BRIDGE_TRIM_HEADROOM_KB"} {
		value, present := os.LookupEnv(name)
		defer func(name, value string, present bool) {
			if present {
				os.Setenv(name, value)
			} else {
				os.Unsetenv(name)
			}
		}(name, value, present)
	}
	os.Setenv("APPLE_BRIDGE_FIXTURE_FREE_KB", "0")
	os.Setenv("APPLE_BRIDGE_MIN_FREE_KB", "1")
	os.Setenv("APPLE_BRIDGE_TRIM_HEADROOM_KB", "1")
	low, err := inspect("low-free-space", "low-space", 0)
	if err != nil {
		return result, err
	}
	_, retainedErr := os.Stat(filepath.Join(directory, low.Key))
	result.Assertions["low_free_space_override_prevents_retention"] = low.CacheBytes == 0 && os.IsNotExist(retainedErr)
	return result, nil
}
