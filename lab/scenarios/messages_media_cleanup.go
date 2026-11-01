package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runMediaCleanupProof(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	directory, err := os.MkdirTemp("/tmp/ablab", "media-cleanup-")
	if err != nil {
		add("private_world", false)
		return e
	}
	defer os.RemoveAll(directory)
	build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	build.WaitDelay = 2 * time.Second
	build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=MessagesVariantChecks")
	if output, err := build.CombinedOutput(); err != nil {
		os.Stderr.Write(output)
		add("native_cleanup_checks_build", false)
		return e
	}
	nativeOutput, err := os.ReadFile("native/.build/native-output-1")
	if err != nil {
		add("native_cleanup_checks_binary", false)
		return e
	}
	binary := filepath.Join(strings.TrimSpace(string(nativeOutput)), "MessagesVariantChecks")
	filename := "ab-run_media.heic"
	fixture := map[string]any{"tag": "[ab-test run_media]", "registration": map[string]any{"scenario": "WEB-MEDIA-VARIANTS", "run_id": "run_media", "baseline_max_row_id": 30, "media_attachment_names": []string{filename}}, "own": []string{"self@example.com"}, "row": map[string]any{"row_id": 31, "attachments": []string{filename}, "peers": []string{"self@example.com"}, "is_from_me": true}, "candidates": []any{map[string]any{"outgoing": true, "values": []string{"file:///private/test/" + filename}}, map[string]any{"outgoing": false, "values": []string{filename}}}}
	run := func(name string, patch func(map[string]any), authorized, target bool) {
		bytes, _ := json.Marshal(fixture)
		var input map[string]any
		json.Unmarshal(bytes, &input)
		patch(input)
		bytes, _ = json.Marshal(input)
		path := filepath.Join(directory, "fixture.json")
		os.WriteFile(path, bytes, 0600)
		started := time.Now()
		output, problem := exec.CommandContext(ctx, binary, "media-cleanup", "--fixtures", path).Output()
		var result struct {
			Authorized bool `json:"authorized"`
			Target     bool `json:"target_verified"`
		}
		decoded := json.Unmarshal(output, &result) == nil
		add(name, problem == nil && decoded && result.Authorized == authorized && result.Target == target)
		e.Latency[name] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	}
	run("attachment_only_target", func(map[string]any) {}, true, true)
	for _, extension := range []string{"mov", "caf"} {
		run("attachment_only_"+extension, func(input map[string]any) {
			name := "ab-run_media." + extension
			input["registration"].(map[string]any)["media_attachment_names"] = []string{name}
			input["row"].(map[string]any)["attachments"] = []string{name}
			input["candidates"] = []any{map[string]any{"description": "Your iMessage, 0-" + name + ", Media, 3:45 PM"}}
		}, true, true)
	}
	run("empty_database_baseline_is_valid", func(input map[string]any) {
		input["registration"].(map[string]any)["baseline_max_row_id"] = 0
		input["row"].(map[string]any)["row_id"] = 1
	}, true, true)
	run("incoming_echo_is_separate_target", func(input map[string]any) { input["row"].(map[string]any)["is_from_me"] = false }, true, true)
	run("native_file_description_outgoing", func(input map[string]any) {
		input["candidates"] = []any{map[string]any{"description": "Your iMessage, 0-" + filename + ", Text Document · 66 bytes, 3:45 PM"}}
	}, true, true)
	run("native_file_description_incoming", func(input map[string]any) {
		input["row"].(map[string]any)["is_from_me"] = false
		input["candidates"] = []any{map[string]any{"description": "Self Test, 0-" + filename + ", Text Document · 66 bytes, 3:45 PM"}}
	}, true, true)
	run("native_file_description_substring_denied", func(input map[string]any) {
		input["candidates"] = []any{map[string]any{"description": "Your iMessage, prefix-" + filename + ", Text Document · 66 bytes, 3:45 PM"}}
	}, true, false)
	run("native_file_description_ambiguous_denied", func(input map[string]any) {
		candidate := map[string]any{"description": "Your iMessage, 0-" + filename + ", Text Document · 66 bytes, 3:45 PM"}
		input["candidates"] = []any{candidate, candidate}
	}, true, false)
	run("photo_preview_filename_with_generic_description", func(input map[string]any) {
		input["candidates"] = []any{map[string]any{"description": "Your iMessage, Photo, 3:45 PM", "values": []string{filename}}}
	}, true, true)
	run("duplicate_copies_require_complete_ownership", func(input map[string]any) {
		candidate := map[string]any{"outgoing": true, "values": []string{filename}}
		input["candidates"] = []any{candidate, candidate}
		input["owned_copies"] = 2
	}, true, true)
	run("unaccounted_duplicate_is_denied", func(input map[string]any) {
		candidate := map[string]any{"outgoing": true, "values": []string{filename}}
		input["candidates"] = []any{candidate, candidate, candidate}
		input["owned_copies"] = 2
	}, true, false)
	run("baseline_data_denied", func(input map[string]any) { input["row"].(map[string]any)["row_id"] = 30 }, false, false)
	run("other_chat_denied", func(input map[string]any) { input["row"].(map[string]any)["peers"] = []string{"other@example.com"} }, false, false)
	run("unregistered_attachment_denied", func(input map[string]any) { input["row"].(map[string]any)["attachments"] = []string{"personal.heic"} }, false, false)
	run("mixed_attachment_bubble_denied", func(input map[string]any) {
		input["row"].(map[string]any)["attachments"] = []string{filename, "personal.heic"}
	}, false, false)
	run("similar_filename_denied", func(input map[string]any) {
		input["candidates"] = []any{map[string]any{"outgoing": true, "values": []string{"prefix-" + filename}}}
	}, true, false)
	run("other_registered_file_is_not_the_target", func(input map[string]any) {
		input["registration"].(map[string]any)["media_attachment_names"] = []string{filename, "ab-run_media.mov"}
		input["candidates"] = []any{map[string]any{"outgoing": true, "values": []string{"ab-run_media.mov"}}}
	}, true, false)
	run("ambiguous_ui_target_denied", func(input map[string]any) {
		input["candidates"] = []any{map[string]any{"outgoing": true, "values": []string{filename}}, map[string]any{"outgoing": true, "values": []string{filename}}}
	}, true, false)
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
