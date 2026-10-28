package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runShortcutSign(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: string(bytes.TrimSpace(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	dir, err := os.MkdirTemp("", "ab-shortcut-sign-")
	add("temporary_build_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	tool := filepath.Join(dir, "shortcuts-tool")
	err = exec.CommandContext(ctx, "go", "build", "-p", "2", "-o", tool, "./tools/shortcuts").Run()
	add("tool_build", err == nil)
	if err != nil {
		return e
	}
	build := func(out string, unsigned bool) ([]byte, error) {
		args := []string{"build", "-source", "packaging/shortcuts/noop.json", "-out", out}
		if unsigned {
			args = append(args, "-unsigned")
		}
		return exec.CommandContext(ctx, tool, args...).Output()
	}
	first := filepath.Join(dir, "first")
	second := filepath.Join(dir, "second")
	_, err = build(first, true)
	add("first_compile", err == nil)
	if err != nil {
		return e
	}
	_, err = build(second, true)
	add("second_compile", err == nil)
	a, err := os.ReadFile(filepath.Join(first, "noop.unsigned.shortcut"))
	add("unsigned_plist_exists", err == nil)
	b, err := os.ReadFile(filepath.Join(second, "noop.unsigned.shortcut"))
	add("deterministic_plist", err == nil && len(a) > 0 && bytes.Equal(a, b))
	raw, err := exec.CommandContext(ctx, "/usr/bin/plutil", "-convert", "json", "-o", "-", filepath.Join(first, "noop.unsigned.shortcut")).Output()
	var plist map[string]any
	decodeErr := json.Unmarshal(raw, &plist)
	actions, ok := plist["WFWorkflowActions"].([]any)
	add("valid_noop_workflow_plist", err == nil && decodeErr == nil && ok && len(actions) == 1)
	e.Metrics["unsigned_sha256"] = fmt.Sprintf("%x", sha256.Sum256(a))
	fixture := filepath.Join(dir, "references.json")
	fixtureSource := `{"version":1,"name":"Synthetic references","actions":[{"key":"text","identifier":"is.workflow.actions.gettext","parameters":{"WFTextActionText":"a < b & c","Number":2,"Flag":true,"Items":["x"]}},{"key":"child","identifier":"is.workflow.actions.runworkflow","parameters":{"WFInput":{"$output":"text"},"Input":{"$input":true},"GroupingIdentifier":{"$group":"branch"}}}]}`
	err = os.WriteFile(fixture, []byte(fixtureSource), 0600)
	add("reference_fixture_written", err == nil)
	_, err = exec.CommandContext(ctx, tool, "build", "-source", fixture, "-out", dir, "-unsigned").Output()
	add("native_parameters_and_references_compile", err == nil)
	raw, err = exec.CommandContext(ctx, "/usr/bin/plutil", "-convert", "json", "-o", "-", filepath.Join(dir, "references.unsigned.shortcut")).Output()
	var referencePlist struct {
		Actions []struct {
			Parameters map[string]any `json:"WFWorkflowActionParameters"`
		} `json:"WFWorkflowActions"`
	}
	decodeErr = json.Unmarshal(raw, &referencePlist)
	validReferences := false
	if err == nil && decodeErr == nil && len(referencePlist.Actions) == 2 {
		first := referencePlist.Actions[0].Parameters
		second := referencePlist.Actions[1].Parameters
		token, _ := second["WFInput"].(map[string]any)
		value, _ := token["Value"].(map[string]any)
		validReferences = first["WFTextActionText"] == "a < b & c" && first["Flag"] == true && first["Number"] == float64(2) && value["OutputUUID"] == first["UUID"] && value["Type"] == "ActionOutput" && token["WFSerializationType"] == "WFTextTokenAttachment"
	}
	add("independent_plist_reference_readback", validReferences)
	for _, invalid := range []string{
		`{"version":2,"name":"Invalid","actions":[]}`,
		`{"version":1,"name":"Invalid","actions":[],"typo":true}`,
		`{"version":1,"name":"Invalid","actions":[{"key":"a","identifier":"x","parameters":{"value":null}}]}`,
		`{"version":1,"name":"Invalid","actions":[{"key":"a","identifier":"x","parameters":{"value":{"$output":"missing"}}}]}`,
		`{"version":1,"name":"Invalid","actions":[{"key":"a","identifier":"x"},{"key":"a","identifier":"x"}]}`,
	} {
		err = os.WriteFile(fixture, []byte(invalid), 0600)
		if err == nil {
			err = exec.CommandContext(ctx, tool, "build", "-source", fixture, "-out", dir, "-unsigned").Run()
		}
		add("invalid_source_rejected", err != nil)
	}
	if level != "real" {
		return e
	}
	signedDir := filepath.Join(dir, "signed")
	raw, err = build(signedDir, false)
	add("shortcuts_sign_succeeds", err == nil)
	if err != nil {
		return e
	}
	var manifest []struct {
		Mode string `json:"mode"`
	}
	err = json.Unmarshal(raw, &manifest)
	add("signing_mode_recorded", err == nil && len(manifest) == 1)
	if len(manifest) == 1 {
		e.Metrics["signing_mode"] = manifest[0].Mode
	}
	signed := filepath.Join(signedDir, "noop.shortcut")
	archive, err := os.ReadFile(signed)
	add("signed_aea_archive", err == nil && len(archive) > 4 && string(archive[:4]) == "AEA1")
	if err != nil || len(manifest) != 1 {
		return e
	}
	err = exec.CommandContext(ctx, "/usr/bin/shortcuts", "sign", "--mode", manifest[0].Mode, "--input", signed, "--output", filepath.Join(dir, "validated.shortcut")).Run()
	add("shortcuts_accepts_signed_archive", err == nil)
	e.Metrics["signed_sha256"] = fmt.Sprintf("%x", sha256.Sum256(archive))
	return e
}
