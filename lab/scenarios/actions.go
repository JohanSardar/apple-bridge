package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runActions(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	d, ok := contracts.Descriptor("shortcuts.actions")
	add("query_registered", ok && d.Kind == contracts.QueryOp && d.Scope == "shortcuts:read")
	if level == "sim" {
		w, err := world.New(world.WithOptions(ctx, world.Options{Scope: "calendar:read,reminders:read,shortcuts:read,system:read"}), level, 0, 0)
		add("public_gateway", err == nil)
		if err != nil {
			os.Stderr.WriteString(err.Error() + "\n")
			return e
		}
		defer w.Close()
		client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
		var result contracts.QueryResult
		ready := w.Await(5*time.Second, func() bool {
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &result) == nil
		})
		add("helper_ready", ready == nil)
		err = client.Call(ctx, "shortcuts.actions", contracts.Query{Args: json.RawMessage(`{}`)}, &result)
		var catalog struct {
			Items []struct {
				Identifier string `json:"identifier"`
			} `json:"items"`
			Complete bool `json:"complete"`
		}
		decode := json.Unmarshal(result.ResultJSON, &catalog)
		add("public_actions", err == nil && decode == nil && catalog.Complete && len(catalog.Items) == 1 && catalog.Items[0].Identifier == "Export")
		err = client.Call(ctx, "shortcuts.actions", contracts.Query{Args: json.RawMessage(`{"path":"/etc/passwd"}`)}, &result)
		add("caller_paths_rejected", err != nil)
	}
	raw, readErr := os.ReadFile("native/.build/native-output-1")
	binary := filepath.Join(strings.TrimSpace(string(raw)), "ActionsChecks")
	info, statErr := os.Stat(binary)
	fresh := readErr == nil && statErr == nil
	if fresh {
		for _, root := range []string{"native/Package.swift", "native/Sources/ActionsChecks", "native/Sources/ShortcutsSource", "native/Sources/SourceSupport"} {
			err := filepath.Walk(root, func(_ string, entry os.FileInfo, err error) error {
				if err != nil {
					return err
				}
				if !entry.IsDir() && entry.ModTime().After(info.ModTime()) {
					fresh = false
				}
				return nil
			})
			fresh = fresh && err == nil
		}
	}
	if !fresh {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=ActionsChecks")
		output, err := build.CombinedOutput()
		add("native_build", err == nil)
		if err != nil {
			os.Stderr.Write(output)
			return e
		}
		raw, readErr = os.ReadFile("native/.build/native-output-1")
		binary = filepath.Join(strings.TrimSpace(string(raw)), "ActionsChecks")
	}
	add("native_output", readErr == nil)
	if readErr != nil {
		return e
	}
	check := exec.CommandContext(ctx, binary, level)
	output, err := check.CombinedOutput()
	add("bounded_metadata_discovery", err == nil)
	if level == "real" && err == nil {
		count, parseErr := strconv.Atoi(strings.TrimSpace(string(output)))
		add("installed_action_count", parseErr == nil && count > 0)
		e.Metrics["action_count"] = count
	}
	if err != nil {
		os.Stderr.Write(output)
	}
	return e
}
