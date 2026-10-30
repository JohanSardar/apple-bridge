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
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runMail(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if s.Feature == "mail.commands" && level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Conditions = map[string]string{"reason": "needs Automation grant AppleBridgeHelper → Mail"}
		return e
	}
	if s.ID == "MAIL-BODY-REAL" {
		return runMailBodyReal(ctx, s, level)
	}
	if s.ID == "MAIL-READ-REAL" {
		return runMailRead(ctx, s, level)
	}
	if s.ID == "MAIL-IDLE" && level == "real" {
		e.Assertions = append(e.Assertions, Assertion{Name: "mail_idle_qualification_pending_T53_and_real_mail_store", Passed: false})
		e.Passed = false
		return e
	}
	if s.ID == "MAIL-REPLICA" {
		return runMailReplica(ctx, s, level)
	}
	binary := ""
	if s.ID == "MAIL-BODY" || s.ID == "MAIL-COMMANDS" || s.ID == "MAIL-IDLE" {
		binary = cachedMailChecks()
	}
	var output []byte
	var err error
	if binary == "" {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=MailChecks")
		output, err = build.CombinedOutput()
		if err == nil {
			directory, readErr := os.ReadFile("native/.build/native-output-1")
			err = readErr
			binary = filepath.Join(strings.TrimSpace(string(directory)), "MailChecks")
		}
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "native_mail_build", Passed: err == nil})
	if err != nil {
		os.Stderr.Write(output)
		e.Passed = false
		return e
	}
	output, err = exec.CommandContext(ctx, binary, s.ID).CombinedOutput()
	name := "native_mail_fixture"
	if s.ID == "MAIL-IDLE" {
		name = "bounded_fixture_idle_no_checkpoints_and_watcher_resumes"
	}
	e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: err == nil})
	e.Passed = err == nil
	if err != nil {
		os.Stderr.Write(output)
	}
	if err == nil && (s.ID == "MAIL-CAPTURE" || s.ID == "MAIL-BODY") {
		var fixture struct {
			Observations []contracts.Observation `json:"observations"`
		}
		err = json.Unmarshal(output, &fixture)
		if err == nil {
			err = mailCaptureWorld(ctx, fixture.Observations)
			if err == nil && s.ID == "MAIL-BODY" {
				err = mailBodyWorld(ctx, fixture.Observations)
			}
		}
		e.Assertions = append(e.Assertions, Assertion{Name: "mail_gateway_capture_and_subject_fts", Passed: err == nil})
		e.Passed = err == nil
	}
	return e
}

func mailCaptureWorld(ctx context.Context, observations []contracts.Observation) error {
	w, err := world.New(ctx, "sim", 1, 0, world.BlobOptions{Scope: "mail:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	if err = w.ControlAction("/dataset", map[string]any{"records": observations}); err != nil {
		return err
	}
	return w.Await(10*time.Second, func() bool {
		records, err := w.Records(w.Gateway)
		counts := map[string]int{}
		for _, record := range records {
			counts[record.Collection]++
		}
		if err != nil || counts["mail.account"] != 3 || counts["mail.mailbox"] != 3 || counts["mail.message"] != 1000 {
			return false
		}
		var found contracts.ReadResult
		_, err = world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Text: "synthetic", Limit: 1000}, &found)
		return err == nil && len(found.Records) == 1000
	})
}

func mailBodyWorld(ctx context.Context, observations []contracts.Observation) error {
	w, err := world.New(ctx, "sim", 1, 0, world.BlobOptions{Scope: "mail:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	if err = w.ControlAction("/dataset", map[string]any{"records": observations}); err != nil {
		return err
	}
	return w.Await(10*time.Second, func() bool {
		records, readErr := w.Records(w.Gateway)
		if readErr != nil {
			return false
		}
		for _, r := range records {
			if r.Collection != "mail.message" {
				continue
			}
			var result contracts.QueryResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/mail.body", contracts.Query{Args: json.RawMessage(`{"message_id":"` + r.ID + `"}`)}, &result)
			var body struct {
				State string `json:"state"`
				Text  string `json:"plain_text"`
			}
			return err == nil && json.Unmarshal(result.ResultJSON, &body) == nil && body.State == "downloaded" && body.Text == "Hello fixture"
		}
		return false
	})
}

func cachedMailChecks() string {
	directory, err := os.ReadFile("native/.build/native-output-1")
	if err != nil {
		return ""
	}
	binary := filepath.Join(strings.TrimSpace(string(directory)), "MailChecks")
	info, err := os.Stat(binary)
	if err != nil {
		return ""
	}
	for _, root := range []string{"native/Package.swift", "native/Sources/MailSource", "native/Sources/MailChecks", "native/Sources/SourceSupport", "packaging/dev/build-native.sh", "packaging/dev/build-native.py", "packaging/dev/sdkroot.sh"} {
		if filepath.Walk(root, func(_ string, entry os.FileInfo, problem error) error {
			if problem != nil {
				return problem
			}
			if entry.ModTime().After(info.ModTime()) {
				return os.ErrInvalid
			}
			return nil
		}) != nil {
			return ""
		}
	}
	return binary
}
