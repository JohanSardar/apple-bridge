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

func runPhotos(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	for _, name := range []string{"photos.search", "photos.original", "photos.thumbnail"} {
		descriptor, found := contracts.Descriptor(name)
		add("descriptor_"+name, found && descriptor.Kind == contracts.QueryOp && descriptor.Scope == "photos:read")
	}
	if s.ID == "PHO-COMMANDS" {
		for _, name := range []string{"photos.import", "photos.create_album", "photos.add_to_album", "photos.remove_from_album", "photos.set_favorite", "photos.delete"} {
			d, found := contracts.Descriptor(name)
			add("descriptor_"+name, found && d.Kind == contracts.CommandOp && d.Scope == "photos:write" && d.Capability == name)
			schema := contracts.OperationSchema(d)
			encoded, err := json.Marshal(schema)
			add("schema_"+name, err == nil && len(encoded) > 0)
		}
		for _, test := range []struct {
			op, args string
			valid    bool
		}{
			{"photos.create_album", `{"name":"Bridge Test"}`, true},
			{"photos.set_favorite", `{"asset":"synthetic","favorite":false}`, true},
			{"photos.set_favorite", `{"asset":"synthetic","favorite":"true"}`, false},
			{"photos.import", `{"data":"YQ==","digest":"bad","kind":"image"}`, false},
			{"photos.delete", `{"asset":"synthetic","force":true}`, false},
		} {
			_, err := contracts.ValidateCommand(contracts.Command{ID: "photos-check", Op: test.op, Args: json.RawMessage(test.args)})
			add("validation_"+test.op, (err == nil) == test.valid)
		}
	}
	group, registered := contracts.GrantsForSource("photos")
	add("source_grants", registered && len(group.Grants) == 2 && contracts.KnownGrant("photos:write"))
	if level == "real" && s.ID == "PHO-IDLE" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "whole_helper_idle", Status: "limited", Detail: "Photos grant missing; whole-Helper CPU qualification blocked by T53"})
		return e
	}
	directory, err := os.ReadFile("native/.build/native-output-1")
	binary := filepath.Join(strings.TrimSpace(string(directory)), "PhotosChecks")
	info, statErr := os.Stat(binary)
	fresh := err == nil && statErr == nil
	if fresh {
		for _, root := range []string{"native/Package.swift", "native/Sources/PhotosSource", "native/Sources/PhotosChecks", "native/Sources/SourceSupport"} {
			walkErr := filepath.Walk(root, func(_ string, item os.FileInfo, walkErr error) error {
				if walkErr != nil {
					return walkErr
				}
				if item.ModTime().After(info.ModTime()) {
					fresh = false
				}
				return nil
			})
			fresh = fresh && walkErr == nil
		}
	}
	if !fresh {
		build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
		build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=PhotosChecks")
		out, buildErr := build.CombinedOutput()
		add("native_build", buildErr == nil)
		if buildErr != nil {
			os.Stderr.Write(out)
			return e
		}
		directory, err = os.ReadFile("native/.build/native-output-1")
		add("native_output", err == nil)
		if err != nil {
			return e
		}
	}
	args := []string{s.ID}
	if level == "real" {
		args = []string{"PHO-READ-REAL"}
	}

	check := exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "PhotosChecks"), args...)
	out, err := check.CombinedOutput()
	add("native_photos_checks", err == nil)
	for _, line := range strings.Split(string(out), "\n") {
		if strings.HasPrefix(line, "idle_cpu_percent=") {
			e.Metrics["idle_cpu_percent"] = strings.TrimPrefix(line, "idle_cpu_percent=")
		}
	}
	if s.ID == "PHO-READ-REAL" || s.ID == "PHO-IMPORT-REAL" || s.ID == "PHO-ORIGINAL-REAL" {
		reason := "PhotoKit count and spot-digest qualification pending"
		for _, line := range strings.Split(string(out), "\n") {
			if strings.HasPrefix(line, "photos_authorization_reason=") && strings.TrimPrefix(line, "photos_authorization_reason=") != "" {
				reason = strings.TrimPrefix(line, "photos_authorization_reason=")
			}
		}
		e.Passed = false
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "photos_readwrite_authorization", Status: "limited", Detail: reason})
	}
	if err != nil {
		os.Stderr.Write(out)
	}
	return e
}
