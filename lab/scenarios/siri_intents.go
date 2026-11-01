package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
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

func runSiriIntents(ctx context.Context, s Scenario, level string) (e Evidence) {
	var cli shortcutProbeCLI = shortcut.CLI{}
	qualified := false
	var stopHost func()
	metadata, metadataErr := os.ReadFile("packaging/app-intents/Metadata.appintents/extract.actionsdata")
	source, sourceErr := os.ReadFile("native/Sources/AppleBridgeHelper/BridgeStatusIntent.swift")
	digest, digestErr := os.ReadFile("packaging/app-intents/source.sha256")
	var actions struct {
		Actions map[string]struct {
			Identifier  string `json:"identifier"`
			OutputFlags *int   `json:"outputFlags"`
		} `json:"actions"`
	}
	parsed := json.Unmarshal(metadata, &actions) == nil && actions.Actions["BridgeStatusIntent"].Identifier == "BridgeStatusIntent"
	flags := actions.Actions["BridgeStatusIntent"].OutputFlags
	noninteractive := parsed && flags != nil && *flags == 0
	emitted := parsed && metadataErr == nil && sourceErr == nil && digestErr == nil && strings.TrimSpace(string(digest)) == fmt.Sprintf("%x", sha256.Sum256(source))
	if level == "sim" {
		cli = &shortcutProbeSimulation{entries: []shortcut.Entry{{Name: "AB Bridge Status"}}, result: shortcut.Result{Data: []byte("Apple Bridge background service is not registered.")}}
		qualified = emitted && noninteractive && rejectsMetadataOnlyHelper(ctx)
	} else {
		artifact, err := os.ReadFile("packaging/shortcuts/AB Bridge Status.shortcut")
		qualified = emitted && noninteractive && err == nil && len(artifact) >= 4 && string(artifact[:4]) == "AEA1"
		if qualified {
			if err := stopStableIntentHosts(ctx); err != nil {
				return Evidence{Scenario: s.ID, Level: level, Assertions: []Assertion{{Name: "previous_status_intent_host_stopped", Passed: false}}}
			}
			_, stop, hostErr := startBridgeIntentHost(ctx)
			if hostErr != nil {
				return Evidence{Scenario: s.ID, Level: level, Assertions: []Assertion{{Name: "current_helper_intent_host_ready", Passed: false}}}
			}
			defer stop()
			stopHost = stop
		}
	}
	e = runShortcutProbe(ctx, s, level, "AB Bridge Status", cli, qualified, validBridgeStatus)
	e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_metadata_matches_source", Passed: emitted})
	if level == "real" {
		e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_appkit_session_dispatch", Passed: e.Passed})
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_returns_without_dialog", Passed: noninteractive})
	e.Passed = e.Passed && emitted && noninteractive
	if level == "sim" {
		scoped := simulatedIntentHostCleanup(ctx)
		e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_host_cleanup_is_scoped", Passed: scoped})
		replaced := simulatedIntentAppReplacement()
		e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_previous_bundle_removed", Passed: replaced})
		e.Passed = e.Passed && scoped && replaced
	}
	if level == "real" && e.Passed && stopHost != nil {
		stopHost()
		baseline := stopStableIntentHosts(ctx) == nil
		e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_cold_baseline_has_no_helper", Passed: baseline})
		e.Passed = e.Passed && baseline
		if !baseline {
			return e
		}
		defer func() {
			cleanup, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			stopped := stopStableIntentHosts(cleanup) == nil
			e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_cold_host_cleaned_up", Passed: stopped})
			e.Passed = e.Passed && stopped
		}()
		result, err := cli.Run(ctx, shortcut.Input{Shortcut: "AB Bridge Status", Timeout: 5 * time.Minute})
		cold := err == nil && result.ExitStatus == 0 && !result.TimedOut && validBridgeStatus(result.Data)
		e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_cold_launch_returns", Passed: cold})
		e.Passed = e.Passed && cold
	}
	return e
}

func rejectsMetadataOnlyHelper(ctx context.Context) bool {
	directory, err := os.MkdirTemp("", "ab-intent-symbol-fixture-")
	if err != nil {
		return false
	}
	defer os.RemoveAll(directory)
	if os.WriteFile(filepath.Join(directory, "nm"), []byte("#!/bin/sh\nprintf 'synthetic_unrelated_symbol\\n'\n"), 0700) != nil {
		return false
	}
	command := exec.CommandContext(ctx, "python3", "packaging/dev/app-intents.py", filepath.Join(directory, "AppleBridgeHelper.app"))
	command.Env = append(os.Environ(), "PATH="+directory+string(os.PathListSeparator)+os.Getenv("PATH"))
	output, err := command.CombinedOutput()
	return err != nil && strings.Contains(string(output), "missing an App Intent implementation")
}

func startBridgeIntentHost(ctx context.Context) (string, func(), error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", nil, err
	}
	parent := filepath.Join(home, "Applications", "AppleBridgeDev")
	if err = os.MkdirAll(parent, 0700); err != nil {
		return "", nil, err
	}
	directory, err := os.MkdirTemp(parent, ".f9-intent-")
	if err != nil {
		return "", nil, err
	}
	app := filepath.Join(directory, "AppleBridgeHelper.app")
	if output, buildErr := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput(); buildErr != nil {
		os.RemoveAll(directory)
		os.Stderr.Write(output)
		return "", nil, buildErr
	}
	installed := filepath.Join(parent, "AppleBridgeHelper.app")
	previous := filepath.Join(directory, "previous.app")
	registrar := "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
	if _, statErr := os.Stat(installed); statErr == nil {
		if err = exec.CommandContext(ctx, registrar, "-u", installed).Run(); err != nil {
			os.RemoveAll(directory)
			return "", nil, fmt.Errorf("Apple Bridge could not unregister its previous development Helper: %w", err)
		}
	}
	if err = replaceBridgeIntentApp(app, installed, previous); err != nil {
		os.RemoveAll(directory)
		return "", nil, err
	}
	if err = exec.CommandContext(ctx, registrar, "-f", installed).Run(); err != nil {
		os.RemoveAll(directory)
		return "", nil, fmt.Errorf("Apple Bridge could not register its current development Helper: %w", err)
	}
	app = installed
	ready := filepath.Join(directory, "pid")
	process := exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "intent-host", strconv.Itoa(os.Getpid()), ready)
	if err = process.Start(); err != nil {
		os.RemoveAll(directory)
		return "", nil, err
	}
	done := make(chan error, 1)
	go func() { done <- process.Wait() }()
	var pid int
	stopped := false
	stop := func() {
		if stopped {
			return
		}
		stopped = true
		if pid > 1 {
			syscall.Kill(pid, syscall.SIGTERM)
			deadline := time.Now().Add(2 * time.Second)
			for time.Now().Before(deadline) && syscall.Kill(pid, 0) == nil {
				time.Sleep(25 * time.Millisecond)
			}
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
			return "", nil, ctx.Err()
		case <-deadline.C:
			stop()
			return "", nil, fmt.Errorf("Apple Bridge could not start its background intent host")
		case err = <-done:
			stop()
			return "", nil, fmt.Errorf("Apple Bridge intent host exited before becoming ready: %v", err)
		case <-tick.C:
			data, readErr := os.ReadFile(ready)
			if readErr == nil {
				pid, err = strconv.Atoi(string(data))
				_, readyErr := os.Stat(ready + ".ready")
				if err == nil && pid > 1 && syscall.Kill(pid, 0) == nil && readyErr == nil {
					return app, stop, nil
				}
			}
		}
	}
}

func replaceBridgeIntentApp(app, installed, previous string) error {
	var err error
	if err = os.Rename(installed, previous); err != nil && !os.IsNotExist(err) {
		return err
	}
	if err = os.Rename(app, installed); err != nil {
		os.Rename(previous, installed)
		return err
	}
	return os.RemoveAll(previous)
}

func stopStableIntentHosts(ctx context.Context) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	return stopIntentExecutables(ctx, filepath.Join(home, "Applications", "AppleBridgeDev", "AppleBridgeHelper.app", "Contents", "MacOS", "AppleBridgeHelper"))
}

func stopIntentExecutables(ctx context.Context, executable string) error {
	pattern := "^" + regexp.QuoteMeta(executable) + "($| )"
	pids := func() ([]int, error) {
		output, err := exec.CommandContext(ctx, "pgrep", "-f", pattern).Output()
		if exit, ok := err.(*exec.ExitError); ok && exit.ExitCode() == 1 {
			return nil, nil
		}
		if err != nil {
			return nil, err
		}
		var result []int
		for _, field := range strings.Fields(string(output)) {
			pid, parseErr := strconv.Atoi(field)
			if parseErr != nil || pid <= 1 || pid == os.Getpid() {
				return nil, fmt.Errorf("Apple Bridge could not identify its development intent host")
			}
			result = append(result, pid)
		}
		return result, nil
	}
	existing, err := pids()
	if err != nil {
		return err
	}
	for _, pid := range existing {
		if err = syscall.Kill(pid, syscall.SIGTERM); err != nil && err != syscall.ESRCH {
			return err
		}
	}
	deadline := time.NewTimer(3 * time.Second)
	defer deadline.Stop()
	for {
		remaining, err := pids()
		if err != nil || len(remaining) == 0 {
			return err
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-deadline.C:
			return fmt.Errorf("Apple Bridge could not stop its previous development intent host before the cold-launch check")
		case <-time.After(50 * time.Millisecond):
		}
	}
}
