package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"os"
	"os/exec"
	"strconv"
	"strings"
	"time"
)

func macRealCommands(ctx context.Context, s Scenario, client scripted.Client, add func(string, bool)) {
	restore := func(run func()) {
		previous := ctx
		cleanup, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		ctx = cleanup
		defer func() { ctx = previous; cancel() }()
		run()
	}
	command := func(op string, args map[string]any) bool {
		raw, _ := json.Marshal(args)
		var out contracts.CommandRecord
		err := client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &out)
		return err == nil && out.State == contracts.CommandObserved && contracts.MacCompletionEvidence(out.Evidence, out.RequestHash)
	}
	script := func(source string) (string, error) {
		b, err := exec.CommandContext(ctx, "/usr/bin/osascript", "-e", source).Output()
		return strings.TrimSpace(string(b)), err
	}
	switch s.ID {
	case "MAC-NOTIFY-REAL":
		raw, _ := json.Marshal(map[string]any{"title": "[ab-test] notification", "body": "[ab-test] qualification"})
		var out contracts.CommandRecord
		err := client.Call(ctx, "mac.notify", contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &out)
		var evidence struct {
			Readback struct {
				Delivered bool `json:"delivered"`
				Cleaned   bool `json:"cleaned"`
			} `json:"readback"`
		}
		var problem *contracts.Error
		if errors.As(err, &problem) {
			fmt.Fprintln(os.Stderr, "notification admission error code:", problem.Code)
		}
		if out.Error != nil {
			fmt.Fprintln(os.Stderr, "notification error code:", out.Error.Code)
		}
		decode := json.Unmarshal(out.Evidence, &evidence)
		fmt.Fprintln(os.Stderr, "notification command state:", out.State)
		add("notification_command_observed", out.State == contracts.CommandObserved)
		add("notification_delivery_readback", decode == nil && evidence.Readback.Delivered)
		add("notification_cleanup_readback", decode == nil && evidence.Readback.Cleaned)
		add("notification_delivered_and_cleaned", err == nil && decode == nil && out.State == contracts.CommandObserved && contracts.MacCompletionEvidence(out.Evidence, out.RequestHash) && evidence.Readback.Delivered && evidence.Readback.Cleaned)

	case "MAC-AUDIO-REAL", "MAC-REPLICA":
		muted, muteErr := script("output muted of (get volume settings)")
		add("original_mute_saved", muteErr == nil)
		if muteErr != nil {
			return
		}
		defer restore(func() {
			add("mute_restored", command("mac.set_mute", map[string]any{"muted": muted == "true"}))
			actual, err := script("output muted of (get volume settings)")
			add("mute_restore_truth", err == nil && actual == muted)
		})
		before, err := script("output volume of (get volume settings)")
		volume, parse := strconv.Atoi(before)
		add("original_volume_saved", err == nil && parse == nil)
		if err != nil || parse != nil {
			return
		}
		defer restore(func() {
			add("volume_restored", command("mac.set_volume", map[string]any{"volume": volume}))
			actual, err := script("output volume of (get volume settings)")
			add("restore_truth", err == nil && actual == before)
		})
		nextVolume := 23
		if volume == nextVolume {
			nextVolume = 24
		}
		add("volume_command_readback", command("mac.set_volume", map[string]any{"volume": nextVolume}))
		actual, err := script("output volume of (get volume settings)")
		add("volume_truth", err == nil && actual == strconv.Itoa(nextVolume))
		add("mute_command_readback", command("mac.set_mute", map[string]any{"muted": muted != "true"}))
		actual, err = script("output muted of (get volume settings)")
		add("mute_truth", err == nil && actual == strconv.FormatBool(muted != "true"))
	case "MAC-APPEARANCE-REAL":
		truth := func() string {
			b, _ := exec.CommandContext(ctx, "/usr/bin/defaults", "read", "-g", "AppleInterfaceStyle").Output()
			if strings.TrimSpace(string(b)) == "Dark" {
				return "dark"
			}
			return "light"
		}
		before := truth()
		next := "dark"
		if before == next {
			next = "light"
		}
		defer restore(func() {
			add("appearance_restored", command("mac.set_appearance", map[string]any{"appearance": before}))
			add("restore_truth", truth() == before)
		})
		add("appearance_command_readback", command("mac.set_appearance", map[string]any{"appearance": next}))
		add("appearance_truth", truth() == next)
	case "MAC-APPS-REAL":
		before, err := script("application id \"com.apple.calculator\" is running")
		add("calculator_not_running", err == nil && before == "false")
		if err != nil || before != "false" {
			return
		}
		frontmostData, frontmostErr := exec.CommandContext(ctx, "/usr/bin/osascript", "-l", "JavaScript", "-e", "ObjC.import('AppKit'); ObjC.unwrap($.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier)").Output()
		frontmost := strings.TrimSpace(string(frontmostData))
		add("frontmost_app_saved", frontmostErr == nil && frontmost != "" && frontmost != "undefined")
		if frontmostErr != nil || frontmost == "" || frontmost == "undefined" {
			return
		}
		launched := command("mac.launch_app", map[string]any{"bundle_id": "com.apple.calculator"})
		defer restore(func() {
			running, err := script("application id \"com.apple.calculator\" is running")
			if err == nil && running == "true" {
				quit := command("mac.quit_app", map[string]any{"bundle_id": "com.apple.calculator"})
				add("calculator_cleanup", quit)
				if !quit {
					_, fallbackErr := exec.CommandContext(ctx, "/usr/bin/osascript", "-l", "JavaScript", "-e", "ObjC.import('AppKit'); const apps = $.NSRunningApplication.runningApplicationsWithBundleIdentifier($('com.apple.calculator')); if (apps.count > 0) apps.objectAtIndex(0).terminate;").Output()
					add("calculator_fallback_cleanup", fallbackErr == nil)
				}
			}
			running, err = script("application id \"com.apple.calculator\" is running")
			add("calculator_restore_truth", err == nil && running == "false")
			add("frontmost_app_restored", command("mac.activate_app", map[string]any{"bundle_id": frontmost}))
		})
		add("calculator_launched", launched)
		if !launched {
			return
		}
		add("original_app_activated_between", command("mac.activate_app", map[string]any{"bundle_id": frontmost}))
		add("calculator_activated", command("mac.activate_app", map[string]any{"bundle_id": "com.apple.calculator"}))
		activeData, activeErr := exec.CommandContext(ctx, "/usr/bin/osascript", "-l", "JavaScript", "-e", "ObjC.import('AppKit'); ObjC.unwrap($.NSWorkspace.sharedWorkspace.frontmostApplication.bundleIdentifier)").Output()
		add("calculator_frontmost_truth", activeErr == nil && strings.TrimSpace(string(activeData)) == "com.apple.calculator")
	case "MAC-KEEPAWAKE-REAL":
		defer restore(func() {
			add("assertion_released", command("mac.keep_awake", map[string]any{"on": false}))
			b, err := exec.CommandContext(ctx, "/usr/bin/pmset", "-g", "assertions").Output()
			add("release_truth", err == nil && !strings.Contains(string(b), "Apple Bridge keep awake"))
		})
		add("assertion_readback", command("mac.keep_awake", map[string]any{"on": true, "duration": 60}))
		b, err := exec.CommandContext(ctx, "/usr/bin/pmset", "-g", "assertions").Output()
		add("assertion_truth", err == nil && strings.Contains(string(b), "Apple Bridge keep awake"))
	default:
		add("mac_real_scenario_supported", false)
	}
}

func macCommandSchemas(add func(string, bool)) {
	expected := map[string]string{"mac.set_volume": "volume", "mac.set_mute": "muted", "mac.set_output_device": "device", "mac.set_appearance": "appearance", "mac.launch_app": "bundle_id", "mac.activate_app": "bundle_id", "mac.quit_app": "bundle_id", "mac.keep_awake": "on", "mac.clipboard_write": "text"}
	for _, descriptor := range contracts.MacDescriptors() {
		schema := contracts.OperationSchema(descriptor)
		args := schema["properties"].(map[string]any)["args"].(map[string]any)
		fields, _ := args["properties"].(map[string]any)
		if descriptor.Name == "mac.recognize_song" {
			field, exists := fields["audio_blob"].(map[string]any)
			required, _ := args["required"].([]string)
			add(descriptor.Name+"_argument_schema", exists && len(fields) == 1 && field["type"] == "string" && field["minLength"] == 4 && field["maxLength"] == 11184812 && field["pattern"] != nil && len(required) == 1 && required[0] == "audio_blob" && args["additionalProperties"] == false)
			continue
		}
		if descriptor.Kind == contracts.QueryOp {
			add(descriptor.Name+"_empty_schema", len(fields) == 0 && args["additionalProperties"] == false)
			continue
		}
		key := expected[descriptor.Name]
		if key == "" {
			continue
		}
		field, exists := fields[key].(map[string]any)
		required, _ := args["required"].([]string)
		top, _ := schema["required"].([]string)
		count := 1
		if descriptor.Name == "mac.keep_awake" {
			count = 2
		}
		add(descriptor.Name+"_argument_schema", exists && field["nullable"] == nil && len(fields) == count && len(required) == 1 && required[0] == key && len(top) == 1 && top[0] == "args")
	}
}
