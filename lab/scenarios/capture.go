package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runCapture(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	directory, directoryErr := os.ReadFile("native/.build/native-output-1")
	fresh := directoryErr == nil && captureChecksFresh(filepath.Join(strings.TrimSpace(string(directory)), "CaptureChecks"))
	var out []byte
	var err error
	if !fresh {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CaptureChecks")
		out, err = build.CombinedOutput()
		add("native_build", err == nil)
		if err != nil {
			os.Stderr.Write(out)
			return e
		}
	}
	directory, err = os.ReadFile("native/.build/native-output-1")
	add("native_output", err == nil)
	if err != nil {
		return e
	}
	out, err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CaptureChecks")).CombinedOutput()
	add("provider_lifecycle_and_artifact_verification", err == nil)
	if err != nil {
		os.Stderr.Write(out)
	}
	publicContext := world.WithOptions(ctx, world.Options{Scope: "calendar:read,reminders:read,system:read,capture:write,mac:write,print:write"})
	var w *world.World
	var worldErr error
	if level == "real" {
		w, worldErr = world.NewRealWithSetup(publicContext, level, 0, world.NativeSetup{IndependentTruth: true})
	} else {
		w, worldErr = world.New(publicContext, level, 0, 0)
	}
	add("public_gateway_start", worldErr == nil)
	if worldErr != nil {
		return e
	}
	defer w.Close()
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	readyErr := w.Await(5*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
	})
	add("public_helper_ready", readyErr == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		client.Protocol = protocol
		for _, op := range []string{"capture.screenshot", "capture.camera_photo", "capture.record_audio", "mac.screen_record", "print.document"} {
			args := json.RawMessage(`{}`)
			if op == "capture.record_audio" || op == "mac.screen_record" {
				args = json.RawMessage(`{"seconds":2}`)
			}
			if op == "print.document" {
				file, pathErr := filepath.Abs("lab/fixtures/vision/fixture.pdf")
				add("print_fixture_path", pathErr == nil)
				args, _ = json.Marshal(map[string]string{"file": file, "printer": "PDF"})
			}
			var command contracts.CommandRecord
			err := client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &command)
			if err != nil || command.State != contracts.CommandObserved {
				code := ""
				var problem *contracts.Error
				if errors.As(err, &problem) {
					code = problem.Code
				}
				if command.Error != nil {
					code = command.Error.Code
				}
				var native struct {
					Code string `json:"native_error_code"`
				}
				json.Unmarshal(command.Evidence, &native)
				fmt.Fprintf(os.Stderr, "%s %s state=%s code=%s native=%s\n", protocol, op, command.State, code, native.Code)
				if e.Conditions == nil {
					e.Conditions = map[string]string{}
				}
				e.Conditions[protocol+"_"+op] = "state=" + string(command.State) + "; code=" + code + "; native=" + native.Code
			}
			add(protocol+"_"+op+"_observed", err == nil && command.State == "observed")
			var result struct {
				Digest string `json:"digest"`
				Size   int64  `json:"size"`
				ID     string `json:"blob_id"`
			}
			json.Unmarshal(command.Evidence, &result)
			req, _ := http.NewRequestWithContext(ctx, "GET", w.Gateway.URL+"/v1/blobs/"+result.Digest+"?record="+result.ID, nil)
			req.Host = w.Gateway.Host
			req.Header.Set("Authorization", "Bearer "+w.Gateway.Token)
			response, downloadErr := http.DefaultClient.Do(req)
			valid := false
			if downloadErr == nil {
				data, readErr := io.ReadAll(response.Body)
				response.Body.Close()
				digest := sha256.Sum256(data)
				valid = response.StatusCode == 200 && readErr == nil && int64(len(data)) == result.Size && hex.EncodeToString(digest[:]) == result.Digest
			}
			add(protocol+"_"+op+"_blob_download", valid)
		}
	}
	return e
}

func captureChecksFresh(binary string) bool {
	info, err := os.Stat(binary)
	if err != nil {
		return false
	}
	fresh := true
	for _, root := range []string{"native/Package.swift", "native/Sources/CaptureSource", "native/Sources/CaptureChecks", "native/Sources/MacSource", "native/Sources/SourceSupport"} {
		err := filepath.Walk(root, func(_ string, item os.FileInfo, err error) error {
			if err != nil {
				return err
			}
			if item.ModTime().After(info.ModTime()) {
				fresh = false
			}
			return nil
		})
		if err != nil {
			return false
		}
	}
	return fresh
}
