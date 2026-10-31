package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runShortcutsRunner(ctx context.Context, s Scenario, level string) Evidence {
	if s.ID == "SC-IDLE" {
		return runShortcutsIdleReal(ctx, s, level)
	}
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	if s.ID == "SC-RUN-REAL" && level != "sim" {
		stop, hostErr := startBridgeIntentHost(ctx)
		add("current_helper_intent_host_ready", hostErr == nil)
		if hostErr != nil {
			return e
		}
		defer stop()
		input := []byte("[ab-test synthetic echo]")
		started := time.Now()
		result, err := (shortcut.CLI{}).Run(ctx, shortcut.Input{Shortcut: "AB Echo", Data: input, Timeout: 5 * time.Minute})
		digest := sha256.Sum256(result.Data)
		e.Metrics["elapsed_ms"] = time.Since(started).Milliseconds()
		e.Metrics["timed_out"] = result.TimedOut
		e.Metrics["exit_status"] = result.ExitStatus
		e.Metrics["output_bytes"] = len(result.Data)
		e.Metrics["output_sha256"] = hex.EncodeToString(digest[:])
		add("bounded_runner_completed", err == nil && result.ExitStatus == 0 && !result.TimedOut)
		add("echo_output_matches", string(result.Data) == string(input))
		if !e.Passed {
			e.Status = "limited"
			e.Conditions = map[string]string{"reason": "AB Echo did not finish or return its input. Open AB Echo in Shortcuts, verify it accepts Shortcut Input without interactive actions, and run it successfully before retrying."}
		}
		return e
	}
	dir, err := os.MkdirTemp("", "ab-shortcuts-fixture-")
	add("fixture_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	executable := filepath.Join(dir, "shortcuts")
	script := `#!/bin/sh
if [ "$1" = list ]; then
 if [ -f "$(dirname "$0")/pack" ]; then
  echo 'AB Echo (00000000-0000-0000-0000-000000000001)'
  echo 'AB Clock (00000000-0000-0000-0000-000000000002)'
  echo 'AB Focus (00000000-0000-0000-0000-000000000003)'
  echo 'AB Pack v1 (00000000-0000-0000-0000-000000000004)'
  exit 0
 fi
 if [ -f "$(dirname "$0")/changed" ]; then
  echo 'Changed (00000000-0000-0000-0000-000000000002)'
 else
  echo 'Synthetic (00000000-0000-0000-0000-000000000001)'
 fi
 exit 0
fi
name="$2"
shift 2
input=""
output=""
while [ "$#" -gt 0 ]; do
 case "$1" in
 --input-path) input="$2"; shift 2;;
 --output-path) output="$2"; shift 2;;
 *) shift;;
 esac
done
case "$name" in
 fail) exit 7;;
 flood) yes synthetic > "$output";;
 stdout-flood) yes synthetic;;
 timeout) echo $$ > "$(dirname "$0")/pid"; sleep 30 & echo $! > "$(dirname "$0")/child"; wait;;
 slow) sleep 16; cat "$input" > "$output";;
 binary) printf '\000\001\377' > "$output";;
 *) cat "$input" > "$output";;
esac
`
	err = os.WriteFile(executable, []byte(script), 0700)
	add("fake_cli", err == nil)
	if err != nil {
		return e
	}
	if s.ID == "SC-RUN-REAL" {
		input := []byte("[ab-test synthetic echo]")
		result, err := (shortcut.CLI{Executable: executable}).Run(ctx, shortcut.Input{Shortcut: "AB Echo", Data: input, Timeout: 20 * time.Second})
		add("bounded_runner_completed", err == nil && result.ExitStatus == 0 && !result.TimedOut)
		add("echo_output_matches", string(result.Data) == string(input))
		return e
	}

	nativeDirectory, directoryErr := os.ReadFile("native/.build/native-output-1")
	fresh := directoryErr == nil && shortcutsChecksFresh(filepath.Join(strings.TrimSpace(string(nativeDirectory)), "ShortcutsChecks"))
	if !fresh {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=ShortcutsChecks")
		buildOutput, buildErr := build.CombinedOutput()
		add("native_shortcuts_build", buildErr == nil)
		if buildErr != nil {
			os.Stderr.Write(buildOutput)
			return e
		}
		nativeDirectory, directoryErr = os.ReadFile("native/.build/native-output-1")
	}
	add("native_shortcuts_output", directoryErr == nil)
	if directoryErr != nil {
		return e
	}
	check := exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(nativeDirectory)), "ShortcutsChecks"), executable, s.ID)
	nativeOutput, nativeErr := check.CombinedOutput()
	add("native_source_fixture", nativeErr == nil)
	if nativeErr != nil {
		os.Stderr.Write(nativeOutput)
	}
	cli := shortcut.CLI{Executable: executable}
	if s.ID == "SC-CAPTURE" {
		first, err := cli.Inventory(ctx, false, "")
		add("initial_inventory", err == nil && len(first) == 1 && first[0].Name == "Synthetic" && first[0].Identifier == "00000000-0000-0000-0000-000000000001")
		err = os.WriteFile(filepath.Join(dir, "changed"), nil, 0600)
		add("change_fixture", err == nil)
		second, err := cli.Inventory(ctx, false, "")
		add("changed_inventory", err == nil && len(second) == 1 && second[0].Name == "Changed" && second[0].Identifier == "00000000-0000-0000-0000-000000000002")
		_, err = shortcut.ParseInventory([]byte("ambiguous name\n"))
		add("malformed_inventory_rejected", err != nil)
		_, err = shortcut.ParseInventory([]byte("a (00000000-0000-0000-0000-000000000001)\nb (00000000-0000-0000-0000-000000000001)\n"))
		add("duplicate_identifiers_rejected", err != nil)
		return e
	}
	descriptor, registered := contracts.Descriptor("shortcuts.run")
	add("public_command_registered", registered && descriptor.Kind == contracts.CommandOp && descriptor.Scope == "shortcuts:write")
	args, _ := json.Marshal(contracts.ShortcutRunArgs{Shortcut: "AB Echo", InputBase64: "c3ludGhldGlj", TimeoutMS: 1000})
	_, validationErr := contracts.ValidateCommand(contracts.Command{ID: "cmd_synthetic", Op: "shortcuts.run", Args: args})
	add("public_command_validated", validationErr == nil)
	for _, input := range []contracts.ShortcutRunArgs{{Shortcut: "AB Echo", InputBase64: "!"}, {Shortcut: "AB Echo", Input: "text", InputBase64: "YQ=="}, {Shortcut: "AB Echo", InputBase64: strings.Repeat("YWFh", 6000)}} {
		raw, _ := json.Marshal(input)
		_, err := contracts.ValidateCommand(contracts.Command{ID: "cmd_synthetic", Op: "shortcuts.run", Args: raw})
		add("invalid_blob_rejected", err != nil)
	}
	publicContext := world.WithOptions(ctx, world.Options{Scope: "calendar:read,reminders:read,shortcuts:read,shortcuts:write,system:read"})
	w, worldErr := world.New(publicContext, level, 0, 0)
	add("public_gateway_start", worldErr == nil)
	if worldErr != nil {
		os.Stderr.WriteString(worldErr.Error() + "\n")
	}
	if worldErr == nil {
		defer w.Close()
		client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
		readyErr := w.Await(5*time.Second, func() bool {
			var version contracts.QueryResult
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &version) == nil
		})
		add("public_helper_ready", readyErr == nil)
		var command contracts.CommandRecord
		callErr := client.Call(ctx, "shortcuts.run", contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &command)
		add("public_runner_completion", callErr == nil && command.State == "observed")
		if callErr != nil {
			os.Stderr.WriteString("public call: " + callErr.Error() + "\n")
		}
		var result struct {
			Output string `json:"output_base64"`
			Digest string `json:"output_sha256"`
			Bytes  int    `json:"output_bytes"`
		}
		json.Unmarshal(command.Evidence, &result)
		add("public_runner_output", result.Output == "c3ludGhldGlj")
		add("public_runner_digest", result.Bytes == 9 && result.Digest == "b3cc0475bb78a5026098858e9889acf666d31062d513d303314eca31d36e72f2")
		slowArgs, _ := json.Marshal(contracts.ShortcutRunArgs{Shortcut: "[ab-test slow runner]", Input: "synthetic", TimeoutMS: 20000})
		slowID := contracts.NewCommandID()
		started := time.Now()
		callErr = client.Call(ctx, "shortcuts.run", contracts.Command{ID: slowID, Args: slowArgs, WaitMS: 15000}, &command)
		add("long_runner_admitted", callErr == nil)
		waitErr := w.Await(10*time.Second, func() bool {
			return client.Call(ctx, "commands.status", contracts.CommandQuery{ID: slowID}, &command) == nil && command.State == "observed"
		})
		e.Metrics["long_runner_elapsed_ms"] = time.Since(started).Milliseconds()
		add("gateway_waits_beyond_15_seconds", waitErr == nil && time.Since(started) > 15*time.Second && command.State == "observed")

	}

	for _, input := range [][]byte{[]byte("[ab-test synthetic]"), []byte("https://example.com/synthetic"), {0, 1, 255}} {
		result, err := cli.Run(ctx, shortcut.Input{Shortcut: "echo", Data: input})
		add("input_output_bytes", err == nil && result.ExitStatus == 0 && string(result.Data) == string(input))
	}
	result, err := cli.Run(ctx, shortcut.Input{Shortcut: "binary"})
	add("binary_output", err == nil && string(result.Data) == string([]byte{0, 1, 255}))
	result, err = cli.Run(ctx, shortcut.Input{Shortcut: "fail"})
	add("nonzero_exit_preserved", err == nil && result.ExitStatus == 7)
	start := time.Now()
	result, err = cli.Run(ctx, shortcut.Input{Shortcut: "timeout", Timeout: 100 * time.Millisecond})
	add("timeout_bounded", errors.Is(err, context.DeadlineExceeded) && result.TimedOut && time.Since(start) < 3*time.Second)
	pidRaw, pidErr := os.ReadFile(filepath.Join(dir, "pid"))
	pid, parseErr := strconv.Atoi(strings.TrimSpace(string(pidRaw)))
	gone := false
	if pidErr == nil && parseErr == nil && pid > 0 {
		gone = errors.Is(syscall.Kill(pid, 0), syscall.ESRCH)
	}
	add("timeout_process_reaped", result.ExitStatus == -1 && gone)
	childRaw, childErr := os.ReadFile(filepath.Join(dir, "child"))
	child, childParse := strconv.Atoi(strings.TrimSpace(string(childRaw)))
	time.Sleep(100 * time.Millisecond)
	add("timeout_descendant_killed", childErr == nil && childParse == nil && errors.Is(syscall.Kill(child, 0), syscall.ESRCH))
	for _, name := range []string{"flood", "stdout-flood"} {
		start := time.Now()
		_, err := cli.Run(ctx, shortcut.Input{Shortcut: name, Timeout: 10 * time.Second})
		add(name+"_bounded_while_running", err != nil && !errors.Is(err, context.DeadlineExceeded) && time.Since(start) < 5*time.Second)
	}
	_, err = cli.Run(ctx, shortcut.Input{Shortcut: "--help"})
	add("option_injection_rejected", err != nil)
	_, err = cli.Run(ctx, shortcut.Input{Shortcut: "echo", Data: make([]byte, shortcut.MaxBytes+1)})
	add("oversized_input_rejected", err != nil)
	return e
}

func shortcutsChecksFresh(path string) bool {
	binary, err := os.Stat(path)
	if err != nil {
		return false
	}
	for _, input := range []string{"native/Package.swift", "native/Sources/ShortcutsChecks", "native/Sources/ShortcutsSource", "native/Sources/SourceSupport"} {
		fresh := true
		err := filepath.Walk(input, func(_ string, info os.FileInfo, err error) error {
			if err != nil {
				return err
			}
			if !info.IsDir() && info.ModTime().After(binary.ModTime()) {
				fresh = false
			}
			return nil
		})
		if err != nil || !fresh {
			return false
		}
	}
	return true
}

func runShortcutsIdleReal(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}}
	w, err := world.NewReal(ctx, level, 0, world.BlobOptions{Scope: "shortcuts:read,system:read"})
	if err != nil {
		e.Assertions = []Assertion{{Name: "world_started", Passed: false}}
		return e
	}
	defer w.Close()
	if w.Await(60*time.Second, func() bool {
		var rows contracts.ReadResult
		return (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "shortcuts.shortcut", Limit: 1}, &rows) == nil && len(rows.Records) > 0
	}) != nil {
		e.Assertions = []Assertion{{Name: "shortcuts_capture_ready", Passed: false}}
		return e
	}
	warmup := time.NewTimer(30 * time.Second)
	defer warmup.Stop()
	select {
	case <-ctx.Done():
		return e
	case <-warmup.C:
	}
	pid, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	cpu := func() (float64, error) {
		raw, err := exec.CommandContext(ctx, "ps", "-p", strings.TrimSpace(string(pid)), "-o", "time=").Output()
		if err != nil {
			return 0, err
		}
		parts := strings.Split(strings.TrimSpace(string(raw)), ":")
		var value float64
		for _, p := range parts {
			n, err := strconv.ParseFloat(p, 64)
			if err != nil {
				return 0, err
			}
			value = value*60 + n
		}
		return value, nil
	}
	if err != nil {
		e.Assertions = []Assertion{{Name: "helper_pid", Passed: false}}
		return e
	}
	before, err := cpu()
	if err != nil {
		return e
	}
	start := time.Now()
	timer := time.NewTimer(5 * time.Minute)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return e
	case <-timer.C:
	}
	after, err := cpu()
	percent := (after - before) / time.Since(start).Seconds() * 100
	e.Metrics["helper_cpu_percent"] = percent
	e.Passed = err == nil && percent >= 0 && percent < 2
	e.Assertions = []Assertion{{Name: "five_minute_helper_cpu_under_two_percent", Passed: e.Passed}}
	return e
}
