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
	app := filepath.Join(directory, "AppleBridgeHelper.app")
	if _, err = exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).CombinedOutput(); err != nil {
		add("signed_helper_build", false)
		return e
	}
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
		output, problem := exec.CommandContext(ctx, filepath.Join(app, "Contents", "MacOS", "AppleBridgeHelper"), "media-cleanup-checks", "--fixtures", path).Output()
		var result struct {
			Authorized bool `json:"authorized"`
			Target     bool `json:"target_verified"`
		}
		decoded := json.Unmarshal(output, &result) == nil
		add(name, problem == nil && decoded && result.Authorized == authorized && result.Target == target)
		e.Latency[name] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	}
	run("attachment_only_target", func(map[string]any) {}, true, true)
	run("empty_database_baseline_is_valid", func(input map[string]any) {
		input["registration"].(map[string]any)["baseline_max_row_id"] = 0
		input["row"].(map[string]any)["row_id"] = 1
	}, true, true)
	run("incoming_echo_is_separate_target", func(input map[string]any) { input["row"].(map[string]any)["is_from_me"] = false }, true, true)
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
