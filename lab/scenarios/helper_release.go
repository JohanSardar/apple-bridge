package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runHelperReleaseCommands(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	directory, err := os.MkdirTemp("", "ab-helper-release-")
	if err != nil {
		add("private_build_directory", false)
		return e
	}
	defer os.RemoveAll(directory)
	apps := map[string]string{}
	keys := map[string]string{}
	for _, mode := range []string{"0", "1"} {
		app := filepath.Join(directory, mode, "AppleBridgeHelper.app")
		command := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app)
		command.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_LAB="+mode)
		output, problem := command.CombinedOutput()
		if problem != nil {
			os.Stderr.Write(output)
			add("signed_helper_build_"+mode, false)
			return e
		}
		add("signed_helper_build_"+mode, true)
		add("signature_valid_"+mode, exec.CommandContext(ctx, "codesign", "--verify", "--strict", app).Run() == nil)
		var proof struct {
			Key string `json:"key"`
		}
		raw, problem := os.ReadFile(filepath.Join(filepath.Dir(app), "AppleBridgeHelper.cache.json"))
		add("cache_attestation_"+mode, problem == nil && json.Unmarshal(raw, &proof) == nil && proof.Key != "")
		keys[mode] = proof.Key
		apps[mode] = filepath.Join(app, "Contents", "MacOS", "AppleBridgeHelper")
	}
	add("lab_and_release_cache_keys_differ", keys["0"] != "" && keys["0"] != keys["1"])
	invoke := func(binary string, args ...string) (string, error) {
		bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
		defer cancel()
		output, err := exec.CommandContext(bounded, binary, args...).CombinedOutput()
		return strings.TrimSpace(string(output)), err
	}
	commands := []string{"reminders-store-blob-fixture", "reminders-store-proof", "reminders-store-flag-proof", "reminders-store-profile", "notes-probe", "functional", "probe", "wire-checks", "media-cleanup-checks"}
	for _, command := range commands {
		output, problem := invoke(apps["0"], command)
		add("release_rejects_"+command, problem != nil && output == "unknownCommand")
		output, problem = invoke(apps["1"], command)
		add("lab_recognizes_"+command, problem != nil && output == "invalidArguments")
	}
	for _, mode := range []string{"0", "1"} {
		for _, command := range []string{"session", "status", "onboard", "selftest"} {
			output, problem := invoke(apps[mode], command)
			add("product_command_"+mode+"_"+command, problem != nil && output == "invalidArguments")
		}
	}
	fixtures := filepath.Join(directory, "wire.json")
	if os.WriteFile(fixtures, []byte("[]"), 0600) != nil {
		add("wire_fixture_written", false)
		return e
	}
	output, problem := invoke(apps["0"], "wire-checks", "--fixtures", fixtures)
	add("release_rejects_valid_lab_fixture", problem != nil && output == "unknownCommand")
	output, problem = invoke(apps["1"], "wire-checks", "--fixtures", fixtures)
	add("lab_executes_wire_fixture", problem == nil && output == "wire fixtures: 0 decoded")
	labOutput, pointerError := os.ReadFile("native/.build/native-output-1")
	if pointerError != nil {
		compile := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		compile.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_LAB=1", "APPLE_BRIDGE_NATIVE_TARGET=AppleBridgeHelper")
		if output, problem := compile.CombinedOutput(); problem != nil {
			os.Stderr.Write(output)
			add("lab_reader_keeps_lab_binary_after_release_build", false)
			return e
		}
		labOutput, pointerError = os.ReadFile("native/.build/native-output-1")
	}
	packaged := filepath.Join(directory, "Apple Bridge.app")
	build := exec.CommandContext(ctx, "packaging/build-app.sh", packaged)
	build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_LAB=1")
	buildOutput, buildError := build.CombinedOutput()
	add("release_package_build", buildError == nil)
	if buildError != nil {
		os.Stderr.Write(buildOutput)
		return e
	}
	add("release_package_signature", exec.CommandContext(ctx, "codesign", "--verify", "--deep", "--strict", packaged).Run() == nil)
	output, problem = invoke(filepath.Join(packaged, "Contents", "MacOS", "AppleBridgeHelper"), "wire-checks", "--fixtures", fixtures)
	add("release_package_overrides_lab_caller", problem != nil && output == "unknownCommand")
	labDirectory := filepath.Dir(mediaTool())
	output, problem = invoke(filepath.Join(labDirectory, "AppleBridgeHelper"), "wire-checks", "--fixtures", fixtures)
	add("lab_reader_keeps_lab_binary_after_release_build", pointerError == nil && labDirectory == strings.TrimSpace(string(labOutput)) && problem == nil && output == "wire fixtures: 0 decoded")
	for _, mode := range []string{"0", "1"} {
		output, problem := exec.CommandContext(ctx, "nm", apps[mode]).Output()
		demangle := exec.CommandContext(ctx, "xcrun", "swift-demangle")
		demangle.Stdin = bytes.NewReader(output)
		symbols, demangleError := demangle.Output()
		for _, name := range []string{"FunctionalProbe", "RemindersStoreProof", "RemindersStoreProfile", "RemindersStoreResponsiveness"} {
			found := strings.Contains(string(symbols), name)
			add("lab_type_"+mode+"_"+name, problem == nil && demangleError == nil && found == (mode == "1"))
		}
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
