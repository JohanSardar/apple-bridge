package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
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
	if s.ID != "CALL-DIAL" && s.ID != "CALL-CAPTURE" && !(s.ID == "CALL-IDLE" && level == "sim") {
		e.Assertions = []Assertion{{Name: "native_level_acceptance_not_implemented", Passed: false}}
		return e
	}
	if s.ID == "CALL-DIAL" {
		d, ok := contracts.Descriptor("phone.dial")
		if !ok || d.Scope != "phone:write" {
			e.Assertions = []Assertion{{Name: "dial_registered", Passed: false}}
			return e
		}
	}
	cmd := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=CallsChecks")
	var err error
	if s.ID != "CALL-DIAL" || !callsDialChecksFresh() {
		var output []byte
		output, err = cmd.CombinedOutput()
		if err != nil {
			os.Stderr.Write(output)
		}
	}
	e.Assertions = []Assertion{{Name: "calls_native_fixture_build", Passed: err == nil}}
	if err == nil {
		directory, readErr := os.ReadFile("native/.build/native-output-1")
		if readErr == nil {
			args := []string{}
			if s.ID == "CALL-DIAL" {
				args = append(args, "--dial")
			}
			if s.ID == "CALL-IDLE" {
				args = append(args, "--idle")
			}
			err = exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(directory)), "CallsChecks"), args...).Run()
		} else {
			err = readErr
		}
		name := "wal_insert_reconcile_delete_partial_scan"
		if s.ID == "CALL-DIAL" {
			name = "fake_dialer_observed_and_indeterminate"
		}
		if s.ID == "CALL-IDLE" {
			name = "five_minute_source_cpu_under_two_percent_and_no_idle_checkpoints"
		}
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: err == nil})
	}
	e.Passed = err == nil
	if s.ID == "CALL-DIAL" && e.Passed {
		return runDialGateway(ctx, s, level, e)
	}
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
	if !awaitIdleCapture(w) {
		e.Assertions = []Assertion{{Name: "messages_capture_and_media_settled", Passed: false}}
		return e
	}
	if w.Await(60*time.Second, func() bool {
		var rows contracts.ReadResult
		return (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "phone.call", Limit: 1}, &rows) == nil && len(rows.Records) > 0
	}) != nil {
		e.Assertions = []Assertion{{Name: "phone_capture_ready", Passed: false}}
		return e
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

func runDialGateway(ctx context.Context, s Scenario, level string, e Evidence) Evidence {
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	w, err := world.New(world.WithOptions(ctx, world.Options{Scope: "calendar:read,reminders:read,phone:read,phone:write,system:read"}), level, 0, 0)
	add("gateway_started", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	add("helper_ready", w.Await(10*time.Second, func() bool {
		var r contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &r) == nil
	}) == nil)
	args := json.RawMessage(`{"number":"+15555550122"}`)
	command := contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}
	var record contracts.CommandRecord
	err = client.Call(ctx, "phone.dial", command, &record)
	add("outgoing_record_observed", err == nil && record.State == "observed" && record.CommitSeq > 0)
	add("fake_no_record_armed", w.ControlAction("/fault", map[string]any{"name": "dial-no-record"}) == nil)
	command.ID = contracts.NewCommandID()
	err = client.Call(ctx, "phone.dial", command, &record)
	add("missing_record_indeterminate", err == nil && record.State == "indeterminate")
	var rows contracts.ReadResult
	_ = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "phone.call", Limit: 100}, &rows)
	count := len(rows.Records)
	add("fake_dialer_reset", w.ControlAction("/fault", map[string]any{"name": ""}) == nil)
	err = client.Call(ctx, "phone.dial", command, &record)
	add("same_id_stays_indeterminate", err == nil && record.State == "indeterminate")
	_ = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "phone.call", Limit: 100}, &rows)
	add("no_retry_effect", len(rows.Records) == count && count == 1)
	authority, authErr := trust.Open(w.Gateway.Home)
	add("agent_authority", authErr == nil)
	if authErr == nil {
		for _, level := range []trust.AgentLevel{trust.AgentRead, trust.AgentApprove, trust.AgentFull} {
			_, token, createErr := authority.CreateAgent("Bridge Test dial", level, []contracts.Grant{"phone:write"})
			add("dial_agent_created", createErr == nil)
			endpoint := w.Gateway
			endpoint.Token = token
			agentClient := scripted.Client{Endpoint: endpoint, Protocol: "rest"}
			command.ID = contracts.NewCommandID()
			callErr := agentClient.Call(ctx, "phone.dial", command, &record)
			if level == trust.AgentRead {
				add("read_agent_denied", callErr != nil)
			} else {
				add("dial_requires_owner_approval", callErr == nil && record.State == contracts.CommandAwaitingApproval)
			}
		}
	}

	return e
}

func callsDialChecksFresh() bool {
	raw, err := os.ReadFile("native/.build/native-output-1")
	if err != nil {
		return false
	}
	root, err := filepath.Abs("native/.build")
	directory := strings.TrimSpace(string(raw))
	if err != nil || !strings.HasPrefix(directory, root+string(os.PathSeparator)) {
		return false
	}
	binary, err := os.Stat(filepath.Join(directory, "CallsChecks"))
	if err != nil {
		return false
	}
	for _, path := range []string{"native/Package.swift", "native/Sources/CallsChecks", "native/Sources/CallsSource", "native/Sources/SourceSupport"} {
		fresh := true
		err := filepath.Walk(path, func(_ string, info os.FileInfo, err error) error {
			if err != nil {
				return err
			}
			if !info.IsDir() && info.ModTime().After(binary.ModTime()) {
				fresh = false
			}
			return nil
		})
		if err != nil || !fresh {
			return false
		}
	}
	return true
}
