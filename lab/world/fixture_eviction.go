package world

import (
	"context"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
	"time"
)

const fixtureCacheLimit int64 = 1536 << 20

func fixtureTreeBytes(directory string) (int64, error) {
	var size int64
	err := filepath.WalkDir(directory, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if !entry.Type().IsRegular() && !entry.IsDir() {
			return nil
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		allocated := info.Size()
		if stat, ok := info.Sys().(*syscall.Stat_t); ok {
			allocated = max(allocated, stat.Blocks*512)
		}
		size += allocated
		return nil
	})
	return size, err
}

func replaceFixtureManifest(directory string, manifest fixtureManifest) error {
	raw, err := json.Marshal(manifest)
	if err != nil {
		return err
	}
	temporary, err := os.CreateTemp(directory, "manifest-*")
	if err != nil {
		return err
	}
	name := temporary.Name()
	temporary.Close()
	os.Remove(name)
	defer os.Remove(name)
	if err = writeFixtureManifest(name, raw); err != nil {
		return err
	}
	if err = os.Rename(name, filepath.Join(directory, "manifest.json")); err != nil {
		return err
	}
	return syncFixtureDirectory(directory)
}

func maintainFixtureCache(ctx context.Context, options FixtureOptions, current string, reserve int64) (int64, error) {
	lock, err := os.OpenFile(filepath.Join(options.Directory, "maintenance.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return 0, err
	}
	defer lock.Close()
	for {
		err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
		if err == nil {
			break
		}
		if err != syscall.EWOULDBLOCK && err != syscall.EAGAIN {
			return 0, err
		}
		select {
		case <-ctx.Done():
			return 0, ctx.Err()
		case <-time.After(25 * time.Millisecond):
		}
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	entries, err := os.ReadDir(options.Directory)
	if err != nil {
		return 0, err
	}
	type item struct {
		name, kind string
		used       time.Time
		bytes      int64
	}
	var candidates []item
	total := int64(0)
	kinds := map[string]int{}
	for _, entry := range entries {
		if !entry.IsDir() {
			continue
		}
		name := entry.Name()
		directory := filepath.Join(options.Directory, name)
		if strings.HasPrefix(name, "building-") {
			info, err := entry.Info()
			if err != nil {
				return 0, err
			}
			if time.Since(info.ModTime()) > options.BuildTimeout {
				key := strings.TrimPrefix(name, "building-")
				if len(key) >= 65 && key[64] == '-' {
					key = key[:64]
				} else {
					key = name
				}
				if err := evictFixtureEntry(options.Directory, name, key, current); err != nil {
					return 0, err
				}
			}
			continue
		}
		if len(name) != 64 {
			continue
		}
		raw, readErr := os.ReadFile(filepath.Join(directory, "manifest.json"))
		var manifest fixtureManifest
		if readErr != nil || json.Unmarshal(raw, &manifest) != nil || !manifest.Metrics.Verified {
			continue
		}
		size, err := fixtureTreeBytes(directory)
		if err != nil {
			return 0, err
		}
		used := manifest.LastUsed
		if used.IsZero() {
			used = manifest.BuiltAt
		}
		candidates = append(candidates, item{name, manifest.Kind, used, size})
		total += size
		kinds[manifest.Kind]++
	}
	pressured := func() (bool, error) {
		free, err := fixtureFreeBytes(options.Directory)
		return free-reserve < fixtureFreeFloor(), err
	}
	sort.Slice(candidates, func(i, j int) bool { return candidates[i].used.Before(candidates[j].used) })
	for _, entry := range candidates {
		low, err := pressured()
		if err != nil {
			return 0, err
		}
		if kinds[entry.kind] <= 2 && total <= options.CacheLimitBytes && !low {
			continue
		}
		if entry.name == current {
			continue
		}
		if err := evictFixtureEntry(options.Directory, entry.name, entry.name, current); err != nil {
			return 0, err
		}
		if _, err := os.Stat(filepath.Join(options.Directory, entry.name)); os.IsNotExist(err) {
			total -= entry.bytes
			kinds[entry.kind]--
		}
	}
	for _, entry := range candidates {
		low, err := pressured()
		if err != nil {
			return 0, err
		}
		if entry.name == current && (total > options.CacheLimitBytes || kinds[entry.kind] > 2 || low) {
			if err := os.RemoveAll(filepath.Join(options.Directory, current)); err != nil {
				return 0, err
			}
			total -= entry.bytes
		}
	}
	return total, nil
}

func evictFixtureEntry(directory, name, key, current string) error {
	if key == current && current != "" {
		return nil
	}
	lock, err := os.OpenFile(filepath.Join(directory, key+".lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		if err == syscall.EWOULDBLOCK || err == syscall.EAGAIN {
			return nil
		}
		return err
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	return os.RemoveAll(filepath.Join(directory, name))
}

func fixtureFreeFloor() int64 {
	value := func(name string, fallback int64) int64 {
		if raw := os.Getenv(name); raw != "" {
			if n, err := strconv.ParseInt(raw, 10, 64); err == nil && n >= 0 && n <= 1<<40 {
				return n * 1024
			}
		}
		return fallback * 1024
	}
	return value("APPLE_BRIDGE_MIN_FREE_KB", 15728640) + value("APPLE_BRIDGE_TRIM_HEADROOM_KB", 1048576)
}

func fixtureFreeBytes(directory string) (int64, error) {
	var stat syscall.Statfs_t
	if err := syscall.Statfs(directory, &stat); err != nil {
		return 0, err
	}
	free := int64(stat.Bavail) * int64(stat.Bsize)
	if raw := os.Getenv("APPLE_BRIDGE_FIXTURE_FREE_KB"); raw != "" {
		if value, err := strconv.ParseInt(raw, 10, 64); err == nil && value >= 0 && value <= 1<<40 {
			free = min(free, value*1024)
		}
	}
	return free, nil
}

func fixtureDiskBytes(directory string) (int64, error) {
	output, err := exec.Command("du", "-sk", directory).Output()
	if err != nil {
		return 0, err
	}
	fields := strings.Fields(string(output))
	if len(fields) == 0 {
		return 0, fmt.Errorf("fixture disk usage missing")
	}
	kib, err := strconv.ParseInt(fields[0], 10, 64)
	return kib * 1024, err
}
