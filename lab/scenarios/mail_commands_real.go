package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func mailRealProof(ctx context.Context, w *world.World, scenario, tag string) (map[string]bool, error) {
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", contracts.NewID("mail-proof")+".json")
	defer os.Remove(report)
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	if err := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "mail-real-checks", scenario, tag, "--report", report).Run(); err != nil {
		return nil, err
	}
	data, err := os.ReadFile(report)
	if err != nil {
		return nil, err
	}
	var proof map[string]bool
	err = json.Unmarshal(data, &proof)
	return proof, err
}

func runMailCommandsReal(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	w, err := world.NewRealWithSetup(ctx, level, 0, world.NativeSetup{FilesystemTruth: true, DevelopmentIdentity: true}, world.BlobOptions{Scope: "mail:read,mail:write,system:read"})
	add("mail_native_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	tag := "[ab-test] " + e.RunID
	defer func() {
		cleanup, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		proof, err := mailRealProof(cleanup, w, "MAIL-CLEANUP", tag)
		add("final_owned_cleanup", err == nil && proof["cleanup_passed"])
	}()
	proof, err := mailRealProof(ctx, w, s.ID, tag)
	add("signed_helper_mail_proof", err == nil && len(proof) > 0)
	names := make([]string, 0, len(proof))
	for name := range proof {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		add(name, proof[name])
	}
	return
}
