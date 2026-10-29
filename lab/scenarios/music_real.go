package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runMusicPlaybackProof(ctx context.Context, w *world.World, s Scenario, e *Evidence) {
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "music-"+e.RunID+".json")
	defer os.Remove(report)
	args := []string{"-n", "-W", "-g", "-a", w.HelperApp, "--args", "music-qualification", "--report", report}
	if s.ID == "MUS-PLAYLIST-REAL" {
		args = append(args, "--playlist")
	}
	probeCtx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	err := exec.CommandContext(probeCtx, "open", args...).Run()
	add("signed_helper_music_proof", err == nil)
	if err != nil {
		return
	}
	var proof struct {
		Status       string          `json:"status"`
		Reason       string          `json:"reason"`
		Assertions   map[string]bool `json:"assertions"`
		BeforeDigest string          `json:"before_digest"`
		AfterDigest  string          `json:"after_digest"`
	}
	data, err := os.ReadFile(report)
	if err == nil {
		err = json.Unmarshal(data, &proof)
	}
	add("music_proof_read", err == nil)
	if err != nil {
		return
	}
	if proof.Status == "limited" {
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "music_control_real", Status: "limited", Detail: proof.Reason})
		return
	}
	add("music_proof_complete", proof.Status == "complete" && len(proof.Assertions) > 0)
	names := make([]string, 0, len(proof.Assertions))
	for name := range proof.Assertions {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		add(name, proof.Assertions[name])
	}
	add("playback_snapshot_digests_match", len(proof.BeforeDigest) == 64 && proof.BeforeDigest == proof.AfterDigest)
	e.Metrics = map[string]any{"playback_before_digest": proof.BeforeDigest, "playback_after_digest": proof.AfterDigest}
}
