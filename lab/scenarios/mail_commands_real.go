package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func mailRealProof(ctx context.Context, w *world.World, scenario, tag string) (map[string]bool, error) {
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", contracts.NewID("mail-proof")+".json")
	defer os.Remove(report)
	ctx, cancel := context.WithTimeout(ctx, 90*time.Second)
	defer cancel()
	launchErr := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "mail-real-checks", scenario, tag, "--report", report).Run()
	data, err := os.ReadFile(report)
	if err != nil {
		return nil, fmt.Errorf("Mail qualification did not produce a report. Open Mail, check that Apple Bridge is running, and try again")
	}
	proof, err := decodeMailProof(data)
	if err == nil && launchErr != nil {
		err = fmt.Errorf("Mail qualification stopped before finishing. Check Mail before trying again")
	}
	return proof, err
}

func decodeMailProof(data []byte) (map[string]bool, error) {
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(data, &fields); err != nil {
		return nil, err
	}
	proof := make(map[string]bool)
	var problems []string
	for _, name := range []string{"command_error", "cleanup_error"} {
		if raw, ok := fields[name]; ok {
			var message string
			if err := json.Unmarshal(raw, &message); err != nil {
				return nil, err
			}
			problems = append(problems, message)
			delete(fields, name)
		}
	}
	for name, raw := range fields {
		var passed bool
		if err := json.Unmarshal(raw, &passed); err != nil {
			return nil, err
		}
		proof[name] = passed
	}
	if len(problems) > 0 {
		return proof, fmt.Errorf("%s", strings.Join(problems, " "))
	}
	return proof, nil
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
		if err != nil {
			if e.Conditions == nil {
				e.Conditions = map[string]string{}
			}
			e.Conditions["cleanup"] = err.Error()
		}
	}()
	proof, err := mailRealProof(ctx, w, s.ID, tag)
	if err != nil {
		e.Conditions = map[string]string{"reason": err.Error()}
	}
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
