package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runWebFixtureSnapshot(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}, Latency: map[string]Latency{}, Assertions: []Assertion{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	directory, err := os.MkdirTemp("", "apple-bridge-fixture-proof-")
	if err != nil {
		add("owned_cache_directory", false)
		return e
	}
	defer os.RemoveAll(directory)
	builds := 0
	prepare := func(w *world.World) (json.RawMessage, error) {
		builds++
		if err := w.ControlAction("/blob-fixture", map[string]any{"size": 4096}); err != nil {
			return nil, err
		}
		if err := w.FlushCaptures(10 * time.Second); err != nil {
			return nil, err
		}
		records, err := w.Records(w.Gateway)
		if err != nil {
			return nil, err
		}
		for _, record := range records {
			refs := contracts.BlobReferences(record.Data)
			if record.Collection != "messages.attachment" || len(refs) == 0 {
				continue
			}
			body, err := snapshotFixtureBlob(ctx, w, refs[0].Digest, record.ID)
			if err != nil {
				return nil, err
			}
			sum := sha256.Sum256(body)
			if hex.EncodeToString(sum[:]) != refs[0].Digest {
				return nil, fmt.Errorf("fixture blob digest")
			}
			return json.Marshal(map[string]string{"record": record.ID, "digest": refs[0].Digest})
		}
		return nil, fmt.Errorf("fixture attachment missing")
	}
	type result struct {
		records  []contracts.Record
		metadata json.RawMessage
		metrics  world.FixtureMetrics
		blob     string
	}
	inspect := func(key string, mutate bool, gateway string) (result, error) {
		w, metadata, metrics, err := world.NewFixture(ctx, world.FixtureOptions{Directory: directory, Key: key, BuildTimeout: time.Minute, GatewayBinary: gateway}, prepare)
		if err != nil {
			return result{}, err
		}
		defer w.Close()
		records, err := w.Records(w.Gateway)
		if err != nil {
			return result{}, err
		}
		var blob map[string]string
		if err = json.Unmarshal(metadata, &blob); err != nil {
			return result{}, err
		}
		body, err := snapshotFixtureBlob(ctx, w, blob["digest"], blob["record"])
		if err != nil {
			return result{}, err
		}
		sum := sha256.Sum256(body)
		if mutate {
			if err = rmPut(w, rmObservation("messages.chat", "copy-only", map[string]any{"display_name": "[ab-test] copy-only", "style": "direct", "participants": []any{}})); err != nil {
				return result{}, err
			}
		}
		return result{records, metadata, metrics, hex.EncodeToString(sum[:])}, nil
	}
	first, err := inspect("version-one", true, "")
	add("first_world_prepared", err == nil)
	if err != nil {
		e.Metrics["error"] = err.Error()
		return e
	}
	second, err := inspect("version-one", false, "")
	add("second_world_prepared", err == nil)
	if err != nil {
		e.Metrics["error"] = err.Error()
		return e
	}
	application := func(records []contracts.Record) []contracts.Record {
		return slices.DeleteFunc(slices.Clone(records), func(record contracts.Record) bool { return record.Collection == "system.source" })
	}
	sourceIDs := func(records []contracts.Record) []string {
		var ids []string
		for _, record := range records {
			if record.Collection == "system.source" {
				ids = append(ids, record.ID)
			}
		}
		slices.Sort(ids)
		return ids
	}
	add("runtime_application_records_equal_and_exclude_copy_mutation", reflect.DeepEqual(application(first.records), application(second.records)))
	add("runtime_source_record_ids_survive_reconnect", reflect.DeepEqual(sourceIDs(first.records), sourceIDs(second.records)))
	add("copied_blob_bytes_and_metadata_equal_fresh_seed", first.blob == second.blob && reflect.DeepEqual(first.metadata, second.metadata))
	info, statErr := os.Stat(directory)
	add("cache_directory_is_owner_only", statErr == nil && info.Mode().Perm() == 0700)
	add("same_key_generates_once", builds == 1 && second.metrics.Hit)
	add("gateway_blob_payloads_are_verified", first.metrics.BlobPayloads > 0 && first.metrics.BlobPayloads == second.metrics.BlobPayloads)
	add("fresh_copy_proof_is_recorded", first.metrics.Verified && first.metrics.FreshDigest != "" && first.metrics.FreshDigest == first.metrics.CopyDigest)
	third, err := inspect("version-two", false, "")
	add("changed_input_prepares_new_template", err == nil && builds == 2 && !third.metrics.Hit)
	alternate := filepath.Join(directory, "alternate-bridge")
	build := exec.CommandContext(ctx, "go", "build", "-tags", "lab", "-p", "2", "-ldflags=-buildid=fixture-snapshot-proof", "-o", alternate, "./cmd/bridge")
	output, buildErr := build.CombinedOutput()
	add("alternate_gateway_binary_builds", buildErr == nil)
	if buildErr != nil {
		e.Metrics["error"] = fmt.Sprintf("alternate gateway: %v: %s", buildErr, output)
		return e
	}
	fourth, err := inspect("version-one", false, alternate)
	add("different_gateway_binary_misses_cache", err == nil && builds == 3 && !fourth.metrics.Hit && fourth.metrics.Key != first.metrics.Key)
	if err != nil {
		e.Metrics["error"] = err.Error()
	}
	chunked, chunkErr := snapshotChunkEquivalence(ctx)
	add("chunked_seed_matches_single_upload_digest", chunkErr == nil && chunked["single"] != "" && chunked["single"] == chunked["chunked"])
	e.Metrics["chunked_seed"] = chunked
	if chunkErr != nil {
		e.Metrics["chunked_error"] = chunkErr.Error()
	}
	eviction, evictionErr := snapshotEvictionProof(ctx, filepath.Join(directory, "eviction"), prepare)
	for name, ok := range eviction.Assertions {
		add(name, ok)
	}
	e.Metrics["disk_eviction"] = eviction
	if evictionErr != nil {
		e.Metrics["eviction_error"] = evictionErr.Error()
		add("eviction_fixture_prepared", false)
	}
	e.Metrics["different_gateway"] = fourth.metrics
	e.Metrics["first"] = first.metrics
	e.Metrics["second"] = second.metrics
	e.Metrics["third"] = third.metrics
	e.Metrics["generator_calls"] = builds
	e.Passed = !slices.ContainsFunc(e.Assertions, func(a Assertion) bool { return !a.Passed })
	return e
}

func snapshotFixtureBlob(ctx context.Context, w *world.World, digest, id string) ([]byte, error) {
	bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	for {
		status, body, err := blobBytes(bounded, w.Gateway, digest, id)
		if err == nil && status == 200 {
			return body, nil
		}
		if err != nil {
			return nil, err
		}
		if status != 503 {
			return nil, fmt.Errorf("fixture blob status %d: %s", status, body)
		}
		select {
		case <-bounded.Done():
			return nil, fmt.Errorf("fixture blob readiness: status %d: %s: %w", status, body, bounded.Err())
		case <-time.After(25 * time.Millisecond):
		}
	}
}
