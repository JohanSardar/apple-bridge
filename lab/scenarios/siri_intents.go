package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

func validBridgeStatus(data []byte) bool {
	switch strings.TrimSpace(string(data)) {
	case "Apple Bridge background service is enabled.",
		"Apple Bridge background service needs approval in System Settings.",
		"Apple Bridge background service is not registered.",
		"Apple Bridge background service configuration was not found.",
		"Apple Bridge background service status is unknown.":
		return true
	}
	return false
}

func runSiriIntents(ctx context.Context, s Scenario, level string) Evidence {
	var cli shortcutProbeCLI = shortcut.CLI{}
	qualified := false
	metadata, metadataErr := os.ReadFile("packaging/app-intents/Metadata.appintents/extract.actionsdata")
	source, sourceErr := os.ReadFile("native/Sources/AppleBridgeHelper/BridgeStatusIntent.swift")
	digest, digestErr := os.ReadFile("packaging/app-intents/source.sha256")
	var actions struct {
		Actions map[string]struct {
			Identifier string `json:"identifier"`
		} `json:"actions"`
	}
	parsed := json.Unmarshal(metadata, &actions) == nil && actions.Actions["BridgeStatusIntent"].Identifier == "BridgeStatusIntent"
	emitted := parsed && metadataErr == nil && sourceErr == nil && digestErr == nil && strings.TrimSpace(string(digest)) == fmt.Sprintf("%x", sha256.Sum256(source))
	if level == "sim" {
		cli = &shortcutProbeSimulation{entries: []shortcut.Entry{{Name: "AB Bridge Status"}}, result: shortcut.Result{Data: []byte("Apple Bridge background service is not registered.")}}
		qualified = emitted
	} else {
		artifact, err := os.ReadFile("packaging/shortcuts/AB Bridge Status.shortcut")
		qualified = emitted && err == nil && len(artifact) >= 4 && string(artifact[:4]) == "AEA1"
		if qualified {
			stop, hostErr := startBridgeIntentHost(ctx)
			if hostErr != nil {
				return Evidence{Scenario: s.ID, Level: level, Assertions: []Assertion{{Name: "current_helper_intent_host_ready", Passed: false}}}
			}
			defer stop()
		}
	}
	e := runShortcutProbe(ctx, s, level, "AB Bridge Status", cli, qualified, validBridgeStatus)
	e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_metadata_matches_source", Passed: emitted})
	e.Passed = e.Passed && emitted
	return e
}

func startBridgeIntentHost(ctx context.Context) (func(), error) {
	directory, err := os.MkdirTemp("", "ab-f9-intent-")
	if err != nil {
		return nil, err
	}
	app := filepath.Join(directory, "AppleBridgeHelper.app")
	if output, buildErr := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput(); buildErr != nil {
		os.RemoveAll(directory)
		os.Stderr.Write(output)
		return nil, buildErr
	}
	ready := filepath.Join(directory, "pid")
	process := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "intent-host", strconv.Itoa(os.Getpid()), ready)
	if err = process.Start(); err != nil {
		os.RemoveAll(directory)
		return nil, err
	}
	done := make(chan error, 1)
	go func() { done <- process.Wait() }()
	var pid int
	stop := func() {
		if pid > 1 {
			syscall.Kill(pid, syscall.SIGTERM)
		}
		process.Process.Kill()
		os.RemoveAll(directory)
	}
	deadline := time.NewTimer(5 * time.Minute)
	defer deadline.Stop()
	tick := time.NewTicker(100 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			stop()
			return nil, ctx.Err()
		case <-deadline.C:
			stop()
			return nil, fmt.Errorf("Apple Bridge could not start its background intent host")
		case err = <-done:
			stop()
			return nil, fmt.Errorf("Apple Bridge intent host exited before becoming ready: %v", err)
		case <-tick.C:
			data, readErr := os.ReadFile(ready)
			if readErr == nil {
				pid, err = strconv.Atoi(string(data))
				if err == nil && pid > 1 && syscall.Kill(pid, 0) == nil {
					return stop, nil
				}
			}
		}
	}
}
