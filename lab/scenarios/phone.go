package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"time"
)

func runPhone(ctx context.Context, s Scenario, level string) Evidence {
	e := runPhoneScenario(ctx, s, level)
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e.GitSHA = strings.TrimSpace(string(sha))
	return e
}

func runPhoneScenario(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}}
	if s.ID == "CALL-IDLE" && level == "real" {
		return runCallsIdleReal(ctx, s, level)
	}
	if s.ID == "CALL-READ-REAL" {
		return runCallsReal(ctx, s, level)
	}
	if s.ID == "CALLS-REPLICA" {
		return runCallsReplica(ctx, s, level)
	}
	if s.ID != "CALL-CAPTURE" && !(s.ID == "CALL-IDLE" && level == "sim") {
		e.Assertions = []Assertion{{Name: "native_level_acceptance_not_implemented", Passed: false}}
		return e
	}
	cmd := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CallsChecks")
	err := cmd.Run()
	e.Assertions = []Assertion{{Name: "calls_native_fixture_build", Passed: err == nil}}
	if err == nil {
		directory, readErr := os.ReadFile("native/.build/native-output-1")
		if readErr == nil {
			args := []string{}
			if s.ID == "CALL-IDLE" {
				args = append(args, "--idle")
			}
			err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CallsChecks"), args...).Run()
		} else {
			err = readErr
		}
		name := "wal_insert_reconcile_delete_partial_scan"
		if s.ID == "CALL-IDLE" {
			name = "five_minute_source_cpu_under_two_percent_and_no_idle_checkpoints"
		}
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: err == nil})
	}
	e.Passed = err == nil
	return e
}

func runCallsReplica(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air", "ubuntu"}}
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "phone:read,system:read"})
	if err != nil {
		e.Assertions = []Assertion{{Name: "world_started", Passed: false}}
		return e
	}
	defer w.Close()
	err = w.ControlAction("/phone", map[string]any{"key": "fixture-call", "data": map[string]any{"direction": "incoming", "handle": "synthetic", "name": "Fixture", "date": time.Now().UTC().Format(time.RFC3339Nano), "duration": 42, "service": "cellular", "answered": true}})
	e.Assertions = append(e.Assertions, Assertion{Name: "fixture_insert", Passed: err == nil})
	converged := w.Await(30*time.Second, func() bool {
		var a, b contracts.ReadResult
		query := contracts.ReadQuery{Collection: "phone.call", Limit: 100}
		x := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.query", query, &a)
		y := (scripted.Client{Endpoint: w.Replicas[0]}).Call(ctx, "records.query", query, &b)
		return x == nil && y == nil && len(a.Records) == 1 && reflect.DeepEqual(recordMultiset(a.Records, true), recordMultiset(b.Records, true))
	}) == nil
	e.Assertions = append(e.Assertions, Assertion{Name: "ubuntu_call_digest_matches", Passed: converged})
	e.Passed = err == nil && converged
	return e
}

func runCallsReal(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.NewReal(ctx, level, 0, world.BlobOptions{Scope: "phone:read,system:read"})
	add("world_started", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	truth, err := exec.CommandContext(ctx, "python3", "lab/truth/phone.py").Output()
	var expected map[string]map[string]any
	if err == nil {
		err = json.Unmarshal(truth, &expected)
	}
	add("independent_truth_read", err == nil)
	if err != nil {
		return e
	}
	matches := func() bool {
		var result contracts.ReadResult
		err := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "phone.call", Limit: 1000, IncludeAliases: true}, &result)
		if err != nil || result.Next != "" {
			return false
		}
		keys := map[string]string{}
		for _, alias := range result.Aliases {
			keys[alias.RecordID] = alias.NativeKey
		}
		actual := map[string]map[string]any{}
		cutoff := time.Now().Add(-30 * 24 * time.Hour).UnixMilli()
		for _, r := range result.Records {
			var data map[string]any
			if json.Unmarshal(r.Data, &data) != nil {
				return false
			}
			at, err := time.Parse(time.RFC3339Nano, fmt.Sprint(data["date"]))
			if err != nil {
				return false
			}
			if at.UnixMilli() < cutoff {
				continue
			}
			data["date"] = float64(at.UnixMilli())
			actual[keys[r.ID]] = data
		}
		if len(actual) != len(expected) || len(expected) < 20 {
			return false
		}
		counts := map[string]int{}
		spots := 0
		for key, want := range expected {
			got, ok := actual[key]
			if !ok {
				return false
			}
			a, _ := json.Marshal(got)
			b, _ := json.Marshal(want)
			if sha256.Sum256(a) != sha256.Sum256(b) {
				return false
			}
			counts[fmt.Sprint(want["direction"])]++
			spots++
		}
		e.Metrics["counts_by_direction"] = counts
		e.Metrics["matching_digests"] = spots
		return true
	}
	add("thirty_day_counts_and_twenty_digests", w.Await(45*time.Second, matches) == nil)
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func runCallsIdleReal(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Metrics: map[string]any{}}
	w, err := world.NewReal(ctx, level, 0, world.BlobOptions{Scope: "phone:read,system:read"})
	if err != nil {
		e.Assertions = []Assertion{{Name: "world_started", Passed: false}}
		return e
	}
	defer w.Close()
	if w.Await(60*time.Second, func() bool {
		var rows contracts.ReadResult
		return (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "phone.call", Limit: 1}, &rows) == nil && len(rows.Records) > 0
	}) != nil {
		e.Assertions = []Assertion{{Name: "phone_capture_ready", Passed: false}}
		return e
	}
	warmup := time.NewTimer(30 * time.Second)
	defer warmup.Stop()
	select {
	case <-ctx.Done():
		return e
	case <-warmup.C:
	}
	pid, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	cpu := func() (float64, error) {
		raw, err := exec.CommandContext(ctx, "ps", "-p", strings.TrimSpace(string(pid)), "-o", "time=").Output()
		if err != nil {
			return 0, err
		}
		parts := strings.Split(strings.TrimSpace(string(raw)), ":")
		var value float64
		for _, p := range parts {
			n, err := strconv.ParseFloat(p, 64)
			if err != nil {
				return 0, err
			}
			value = value*60 + n
		}
		return value, nil
	}
	if err != nil {
		e.Assertions = []Assertion{{Name: "helper_pid", Passed: false}}
		return e
	}
	before, err := cpu()
	if err != nil {
		return e
	}
	start := time.Now()
	timer := time.NewTimer(5 * time.Minute)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return e
	case <-timer.C:
	}
	after, err := cpu()
	percent := (after - before) / time.Since(start).Seconds() * 100
	e.Metrics["helper_cpu_percent"] = percent
	e.Passed = err == nil && percent >= 0 && percent < 2
	e.Assertions = []Assertion{{Name: "five_minute_helper_cpu_under_two_percent", Passed: e.Passed}}
	return e
}
