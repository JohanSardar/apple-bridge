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
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewReal(ctx, level, 1)
	} else {
		w, err = world.New(ctx, level, 1, 0)
	}
	add("isolated_world_ready", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	var fixture struct {
		Digest string `json:"digest"`
		Size   int64  `json:"size"`
		Title  string `json:"title"`
	}
	err = w.BlobControl(map[string]string{"action": "relay-fixture"}, &fixture)
	add("synthetic_blob_staged", err == nil)
	if err != nil {
		raw, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "run", "relay-blob-error.log"))
		e.Assertions[len(e.Assertions)-1].Detail = err.Error() + ": " + string(raw)
		return e
	}
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
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
