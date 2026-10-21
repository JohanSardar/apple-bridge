package scenarios

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayNativeFunctionalChecks(w *world.World, e *Evidence, add func(string, bool), tag string) {
	title := tag + " forwarded command"
	commandInput := relayProbeInput(w)
	commandInput["Title"] = title
	var command struct {
		Create     bool `json:"create_observed"`
		CreateRead bool `json:"create_read_your_write"`
		Delete     bool `json:"delete_observed"`
		DeleteRead bool `json:"delete_read_your_write"`
	}
	err := w.RelayExchange("command", commandInput, func() (any, error) {
		truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": title})
		if err != nil {
			return nil, err
		}
		records, _ := truth["records"].([]any)
		if len(records) != 1 {
			return nil, fmt.Errorf("forwarded create not independently visible")
		}
		return map[string]bool{"truth_verified": true}, nil
	}, &command)
	add("forwarded_create_and_delete_are_observed_and_read_your_write", err == nil && command.Create && command.CreateRead && command.Delete && command.DeleteRead)
	if err != nil {
		return
	}
	truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": title})
	records, _ := truth["records"].([]any)
	add("forwarded_delete_independent_native_absence", err == nil && len(records) == 0)
	relayNativeBlobCheck(w, e, add)
}

func relayNativeBlobCheck(w *world.World, e *Evidence, add func(string, bool)) bool {
	var fixture struct {
		Digest string `json:"digest"`
		Size   int64  `json:"size"`
		Title  string `json:"title"`
	}
	started := time.Now()
	err := w.BlobControl(map[string]string{"action": "relay-fixture"}, &fixture)
	e.Metrics["nominal_blob_stage_elapsed_ms"] = float64(time.Since(started)) / float64(time.Millisecond)
	add("synthetic_blob_staged_in_isolated_native_gateway", err == nil)
	if err != nil {
		os.WriteFile(filepath.Join(w.Dir, "relay-blob-call-error.log"), []byte(err.Error()), 0600)
		return false
	}
	blobInput := relayProbeInput(w)
	blobInput["Title"] = fixture.Title
	blobInput["Digest"] = fixture.Digest
	blobInput["Size"] = fixture.Size
	var blob struct {
		Bytes   int64 `json:"bytes"`
		Matches bool  `json:"digest_matches"`
	}
	err = w.RelayProbe("blob", blobInput, &blob)
	add("blob_bytes_through_public_relay_match_gateway_digest", err == nil && blob.Matches && blob.Bytes == fixture.Size)
	e.Metrics["relay_blob_bytes"] = blob.Bytes
	return err == nil
}
