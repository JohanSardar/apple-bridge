package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func messageSendVariant(id string) string {
	return map[string]string{"MSG-AUDIO-MESSAGE": "audio", "MSG-FORMATTING-SEND": "formatting", "MSG-MENTION-SEND": "mention", "MSG-SEND-EFFECT": "effect", "MSG-SEND-LATER": "schedule"}[id]
}

func runMessagesVariantReal(ctx context.Context, s Scenario, e Evidence) (result Evidence) {
	result = e
	before, err := cleanupSnapshot(ctx, "messages")
	if err != nil {
		result.Status = "limited"
		result.Conditions = map[string]string{"reason": "needs Messages Full Disk Access"}
		return
	}
	audit, err := newRunCleanup(e.RunID, before)
	if err != nil {
		return
	}
	defer func() { finishCleanupAudit(context.Background(), audit, "messages", &result) }()
	dir, err := os.MkdirTemp("", "ab-message-variant-")
	if err != nil {
		return
	}
	defer os.RemoveAll(dir)
	app := filepath.Join(dir, "AppleBridgeHelper.app")
	build := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app)
	build.WaitDelay = 2 * time.Second
	output, err := build.CombinedOutput()
	if err != nil {
		os.Stderr.Write(output)
		result.Assertions = append(result.Assertions, Assertion{Name: "signed_helper_build", Passed: false})
		return
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "variant-"+e.RunID+".json")
	defer os.Remove(report)
	err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "messages-variant-proof", messageSendVariant(s.ID), "--report", report).Run()
	raw, readErr := os.ReadFile(report)
	var proof struct {
		Passed  bool   `json:"passed"`
		Limited bool   `json:"limited"`
		Reason  string `json:"reason"`
		Cleanup *bool  `json:"cleanup_passed"`
		Writes  int    `json:"writes_submitted"`
	}
	if err != nil || readErr != nil || json.Unmarshal(raw, &proof) != nil {
		result.Assertions = append(result.Assertions, Assertion{Name: "native_variant_report", Passed: false})
		return
	}
	result.Metrics = map[string]any{"writes_submitted": proof.Writes}
	if proof.Cleanup != nil {
		result.Assertions = append(result.Assertions, Assertion{Name: "owned_messages_cleanup", Passed: *proof.Cleanup})
	}
	result.Passed = proof.Passed
	if proof.Limited {
		result.Status = "limited"
		result.Conditions = map[string]string{"reason": proof.Reason}
	} else {
		result.Assertions = append(result.Assertions, Assertion{Name: "sent_variant_attributes_readback", Passed: proof.Passed})
	}
	return
}

func runMessagesVariantReadback(ctx context.Context, kind string, add func(string, bool)) {
	directory, _ := os.ReadFile("native/.build/native-output-1")
	binary := filepath.Join(strings.TrimSpace(string(directory)), "MessagesVariantChecks")
	if !messagesVariantChecksFresh(binary) {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.WaitDelay = 2 * time.Second
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=MessagesVariantChecks")
		output, err := build.CombinedOutput()
		add("native_readback_checks_build", err == nil)
		if err != nil {
			os.Stderr.Write(output)
			return
		}
		directory, _ = os.ReadFile("native/.build/native-output-1")
		binary = filepath.Join(strings.TrimSpace(string(directory)), "MessagesVariantChecks")
	}
	output, err := exec.CommandContext(ctx, binary, kind).Output()
	var checks map[string]bool
	ok := err == nil && json.Unmarshal(output, &checks) == nil && len(checks) > 0
	add("native_readback_checks", ok)
	if ok {
		for name, passed := range checks {
			add(name, passed)
		}
	}
}

func messagesVariantChecksFresh(binary string) bool {
	info, err := os.Stat(binary)
	if err != nil {
		return false
	}
	for _, root := range []string{"native/Package.swift", "native/Sources/MessagesVariantChecks", "native/Sources/MessagesSource", "native/Sources/SourceSupport", "native/Sources/Decoders", "packaging/dev/build-native.sh", "packaging/dev/build-native.py"} {
		if filepath.Walk(root, func(_ string, entry os.FileInfo, problem error) error {
			if problem != nil {
				return problem
			}
			if entry.ModTime().After(info.ModTime()) {
				return os.ErrInvalid
			}
			return nil
		}) != nil {
			return false
		}
	}
	return true
}
