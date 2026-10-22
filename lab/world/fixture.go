package world

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

type FixtureOptions struct {
	Kind            string
	CacheLimitBytes int64
	Directory       string
	Key             string
	BuildTimeout    time.Duration
	GatewayBinary   string
}

type FixtureMetrics struct {
	DiskBytes    int64  `json:"disk_bytes"`
	EntryBytes   int64  `json:"entry_bytes"`
	CacheBytes   int64  `json:"cache_bytes"`
	BlobPayloads int    `json:"blob_payloads"`
	Key          string `json:"key"`
	Hit          bool   `json:"cache_hit"`
	BuildMS      int64  `json:"build_ms"`
	CopyMS       int64  `json:"copy_ms"`
	Records      int    `json:"records"`
	FreshDigest  string `json:"fresh_digest"`
	CopyDigest   string `json:"copy_digest"`
	Verified     bool   `json:"verified"`
	CopyMethod   string `json:"copy_method"`
}

type fixtureManifest struct {
	Kind     string          `json:"kind"`
	LastUsed time.Time       `json:"last_used"`
	Version  int             `json:"version"`
	BuiltAt  time.Time       `json:"built_at"`
	Metadata json.RawMessage `json:"metadata"`
	Metrics  FixtureMetrics  `json:"metrics"`
}

type fixtureRestoreKey struct{}
type fixtureRestore struct {
	Directory string
	Verify    func(string) error
}

func NewFixture(ctx context.Context, options FixtureOptions, prepare func(*World) (json.RawMessage, error)) (*World, json.RawMessage, FixtureMetrics, error) {
	if options.Key == "" || prepare == nil {
		return nil, nil, FixtureMetrics{}, fmt.Errorf("fixture inputs required")
	}
	if options.Kind == "" {
		options.Kind = "default"
	}
	if options.CacheLimitBytes <= 0 || options.CacheLimitBytes > fixtureCacheLimit {
		options.CacheLimitBytes = fixtureCacheLimit
	}
	if options.BuildTimeout <= 0 {
		options.BuildTimeout = 6 * time.Minute
	}
	buildCtx, cancel := context.WithTimeout(ctx, options.BuildTimeout)
	defer cancel()
	if options.GatewayBinary == "" {
		options.GatewayBinary, _ = filepath.Abs("bin/bridge")
	}
	simulator, err := os.Executable()
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	key, err := fixtureFingerprint(options.Kind+"\x00"+options.Key, options.GatewayBinary, simulator)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if err = buildCtx.Err(); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if options.Directory == "" {
		root, err := os.UserCacheDir()
		if err != nil {
			return nil, nil, FixtureMetrics{}, err
		}
		options.Directory = filepath.Join(root, "apple-bridge", "lab-fixtures-v1")
	}
	if err = os.MkdirAll(options.Directory, 0700); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if err = os.Chmod(options.Directory, 0700); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if _, err = maintainFixtureCache(buildCtx, options, "", 0); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	lock, err := os.OpenFile(filepath.Join(options.Directory, key+".lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	defer lock.Close()
	for {
		err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB)
		if err == nil {
			break
		}
		if !errors.Is(err, syscall.EWOULDBLOCK) && !errors.Is(err, syscall.EAGAIN) {
			return nil, nil, FixtureMetrics{}, err
		}
		select {
		case <-buildCtx.Done():
			return nil, nil, FixtureMetrics{}, buildCtx.Err()
		case <-time.After(25 * time.Millisecond):
		}
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	destination := filepath.Join(options.Directory, key)
	var manifest fixtureManifest
	if raw, readErr := os.ReadFile(filepath.Join(destination, "manifest.json")); readErr == nil && json.Unmarshal(raw, &manifest) == nil && manifest.Version == 1 && manifest.Metrics.Key == key && manifest.Metrics.Verified {
		started := time.Now()
		w, err := newWorld(context.WithValue(ctx, fixtureRestoreKey{}, fixtureRestore{Directory: filepath.Join(destination, "data")}), "sim", 0, 0, options.GatewayBinary, BlobOptions{Scope: "admin"})
		if err != nil {
			return nil, nil, FixtureMetrics{}, err
		}
		if err = buildCtx.Err(); err != nil {
			w.Close()
			return nil, nil, FixtureMetrics{}, err
		}
		manifest.LastUsed = time.Now().UTC()
		if err = replaceFixtureManifest(destination, manifest); err != nil {
			w.Close()
			return nil, nil, FixtureMetrics{}, err
		}
		cacheBytes, err := maintainFixtureCache(buildCtx, options, key, 0)
		if err != nil {
			w.Close()
			return nil, nil, FixtureMetrics{}, err
		}
		metrics := manifest.Metrics
		metrics.CacheBytes = cacheBytes
		metrics.Hit = true
		metrics.BuildMS = 0
		metrics.CopyMS = time.Since(started).Milliseconds()
		metrics.CopyMethod = w.fixtureCopyMethod
		return w, manifest.Metadata, metrics, nil
	}
	if err = os.RemoveAll(destination); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	started := time.Now()
	source, err := newWorld(buildCtx, "sim", 0, 0, options.GatewayBinary, BlobOptions{Scope: "admin"})
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	defer func() {
		if source != nil {
			source.Close()
		}
	}()
	metadata, err := prepare(source)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	for i := len(source.processes) - 1; i >= 0; i-- {
		source.processes[i].Stop()
	}
	fresh, count, err := fixtureRecordDigest(buildCtx, source.Gateway.Home)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	blobs, err := fixtureHelperDigest(source.Dir)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	payloads, payloadCount, err := fixturePayloadDigest(filepath.Join(source.Gateway.Home, "blobs"))
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	staging, err := os.MkdirTemp(options.Directory, "building-"+key+"-")
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	defer os.RemoveAll(staging)
	if _, err = copyFixtureTree(source.Dir, filepath.Join(staging, "data"), true); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	source.Close()
	source = nil
	copyStarted := time.Now()
	copied := ""
	copiedCount := 0
	verify := func(directory string) error {
		var err error
		copied, copiedCount, err = fixtureRecordDigest(buildCtx, filepath.Join(directory, "gateway"))
		if err != nil {
			return err
		}
		copiedBlobs, err := fixtureHelperDigest(directory)
		if err != nil {
			return err
		}
		copiedPayloads, copiedPayloadCount, err := fixturePayloadDigest(filepath.Join(directory, "gateway", "blobs"))
		if err != nil {
			return err
		}
		if fresh != copied || count != copiedCount || blobs != copiedBlobs || payloads != copiedPayloads || payloadCount != copiedPayloadCount {
			return fmt.Errorf("fixture copy differs: records=%t(%d/%d) helper_state=%t gateway_blobs=%t(%d/%d)", fresh == copied, count, copiedCount, blobs == copiedBlobs, payloads == copiedPayloads, payloadCount, copiedPayloadCount)
		}
		return nil
	}
	w, err := newWorld(context.WithValue(ctx, fixtureRestoreKey{}, fixtureRestore{Directory: filepath.Join(staging, "data"), Verify: verify}), "sim", 0, 0, options.GatewayBinary, BlobOptions{Scope: "admin"})
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	accepted := false
	defer func() {
		if !accepted {
			w.Close()
		}
	}()
	copyMS := time.Since(copyStarted).Milliseconds()
	if err = buildCtx.Err(); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	metrics := FixtureMetrics{BlobPayloads: payloadCount, Key: key, BuildMS: time.Since(started).Milliseconds(), CopyMS: copyMS, Records: count, FreshDigest: fresh, CopyDigest: copied, Verified: true, CopyMethod: w.fixtureCopyMethod}
	metrics.EntryBytes, err = fixtureTreeBytes(filepath.Join(staging, "data"))
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	metrics.DiskBytes, err = fixtureDiskBytes(filepath.Join(staging, "data"))
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	manifest = fixtureManifest{Kind: options.Kind, LastUsed: time.Now().UTC(), Version: 1, BuiltAt: time.Now().UTC(), Metadata: metadata, Metrics: metrics}
	raw, err := json.Marshal(manifest)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if err = writeFixtureManifest(filepath.Join(staging, "manifest.json"), raw); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if err = os.Rename(staging, destination); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	if err = syncFixtureDirectory(options.Directory); err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	metrics.CacheBytes, err = maintainFixtureCache(buildCtx, options, key, metrics.EntryBytes)
	if err != nil {
		return nil, nil, FixtureMetrics{}, err
	}
	accepted = true
	return w, metadata, metrics, nil
}

func writeFixtureManifest(path string, raw []byte) error {
	file, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	_, writeErr := file.Write(raw)
	if writeErr == nil {
		writeErr = file.Sync()
	}
	closeErr := file.Close()
	if writeErr != nil {
		return writeErr
	}
	if closeErr != nil {
		return closeErr
	}
	return syncFixtureDirectory(filepath.Dir(path))
}

func syncFixtureDirectory(path string) error {
	directory, err := os.Open(path)
	if err != nil {
		return err
	}
	defer directory.Close()
	return directory.Sync()
}
