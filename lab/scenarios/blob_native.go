package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runNativeBlobStage(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	concurrent := strings.HasPrefix(s.ID, "CORE-BLOB-STAGE-")
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewReal(ctx, level, 1)
	} else if concurrent {
		w, err = world.New(ctx, level, 1, 0, world.BlobOptions{})
	} else {
		w, err = world.New(ctx, level, 1, 0)
	}
	add("isolated_world_ready", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	if concurrent {
		size := 4 << 20
		input := map[string]any{"size": size, "slow_ms": 1000}
		if s.ID == "CORE-BLOB-STAGE-SAME-DIGEST" {
			size = 172032
			input["size"], input["relay_fixture"] = size, true
		}
		err = w.ControlAction("/blob-fixture", input)
		add("slow_native_blob_fixture", err == nil)
		if err != nil {
			return e
		}
		err = w.Await(10*time.Second, func() bool {
			progress, err := blobProgress(ctx, w.Gateway)
			for _, entry := range progress.Entries {
				if entry.State == "pending" && entry.Received > 0 && entry.Received < int64(size) {
					return err == nil
				}
			}
			return false
		})
		add("native_transfer_in_progress", err == nil)
		if err != nil {
			return e
		}
	}
	var fixture struct {
		Digest string `json:"digest"`
		Size   int64  `json:"size"`
		Title  string `json:"title"`
	}
	staged := time.Now()
	err = w.BlobControl(map[string]string{"action": "relay-fixture"}, &fixture)
	e.Latency["local_blob_stage"] = stats([]float64{float64(time.Since(staged)) / float64(time.Millisecond)}, 0)
	add("synthetic_blob_staged", err == nil)
	if err != nil {
		raw, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "run", "relay-blob-error.log"))
		e.Assertions[len(e.Assertions)-1].Detail = err.Error() + ": " + string(raw)
		return e
	}
	if s.ID == "CORE-BLOB-STAGE-SAME-DIGEST" {
		err = w.Await(5*time.Second, func() bool {
			_, err := os.Stat(filepath.Join(w.Gateway.Home, "blobs", "staging", fixture.Digest+".part"))
			return os.IsNotExist(err)
		})
		add("same_digest_partial_transfer_retired", err == nil)
		if err != nil {
			return e
		}
	}
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		var recordID string
		err = w.Await(10*time.Second, func() bool {
			records, err := w.Records(endpoint)
			if err != nil {
				return false
			}
			for _, record := range records {
				if recordTitle(record) == fixture.Title {
					recordID = record.ID
					return true
				}
			}
			return false
		})
		add("fixture_record_visible", err == nil)
		if err != nil {
			return e
		}
		if s.ID == "CORE-BLOB-STAGE-DURING-TRANSFER" && index > 0 {
			continue
		}
		err = w.Await(10*time.Second, func() bool {
			status, payload, err := blobBytes(ctx, endpoint, fixture.Digest, recordID)
			digest := sha256.Sum256(payload)
			return err == nil && status == 200 && int64(len(payload)) == fixture.Size && hex.EncodeToString(digest[:]) == fixture.Digest
		})
		add("authorized_blob_bytes_match", err == nil)
		if err != nil {
			return e
		}
	}
	e.Passed = true
	return e
}
