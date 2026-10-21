package scenarios

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayNativeBlobChecks(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	if !relayNativeBlobCheck(w, e, add) || ctx.Err() != nil {
		return
	}
	tag := "[ab-test " + contracts.NewID("relay-blob") + "]"
	_, err := w.Truth("fixture", map[string]any{"action": "create_list", "list": "Bridge Test", "tag": tag})
	add("blob_burst_fixture_list", err == nil)
	if err != nil {
		return
	}
	defer func() {
		_, err := w.RecoverTruth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": tag})
		add("blob_burst_tagged_fixture_cleanup", err == nil)
	}()
	burst, err := w.Truth("fixture", map[string]any{"action": "burst", "list": "Bridge Test", "tag": tag, "count": relayLiveSamples})
	effects, _ := burst["effects"].([]any)
	add("native_burst_before_timed_blob_stage", err == nil && len(effects) == relayLiveSamples)
	e.Metrics["blob_context_burst_effects"] = len(effects)
	if err != nil || len(effects) != relayLiveSamples {
		return
	}
	relayPostBurstBlobCheck(w, e)
	e.Metrics["latency_verification"] = "retained full proof; blocked on LIVE-FIX"
}
