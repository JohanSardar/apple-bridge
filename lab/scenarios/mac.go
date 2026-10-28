package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"
)

func runMac(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC()}
	add := func(n string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: n, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	ctx = world.WithOptions(ctx, world.Options{Scope: "mac:read,mac:write,system:read"})
	var w *world.World
	var err error
	if level == "real" || s.ID == "MAC-REPLICA" {
		w, err = world.NewRealWithSetup(ctx, level, 1, world.NativeSetup{IndependentTruth: true})
	} else {
		w, err = world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "mac:read,mac:write,reminders:read,system:read"})
	}
	add("world_started", err == nil)
	if err != nil {
		fmt.Fprintln(os.Stderr, "mac world startup:", err)
		return
	}
	defer w.Close()

	endpoint := w.Gateway
	if s.ID == "MAC-REPLICA" && len(w.Replicas) > 0 {
		endpoint = w.Replicas[0]
	}
	endpoint.Token, err = w.ScopedToken(endpoint, "mac:read,mac:write,system:read")
	add("scope_granted", err == nil)
	client := scripted.Client{Endpoint: endpoint}
	err = w.Await(20*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
	})
	add("helper_ready", err == nil)
	if err != nil {
		return
	}
	if s.ID != "MAC-COMMANDS" && s.ID != "MAC-CAPTURE" && s.ID != "MAC-IDLE" && s.ID != "MAC-POWER-REAL" {
		if s.ID == "MAC-CLIPBOARD-REAL" {
			macClipboardCommands(ctx, client, w, true, add)
		} else {
			macRealCommands(ctx, s, client, add)
		}
		return
	}
	if s.ID == "MAC-COMMANDS" {
		macCommandSchemas(add)
		for _, test := range []struct {
			op   string
			args map[string]any
		}{
			{"mac.set_volume", map[string]any{"volume": 23}},
			{"mac.set_mute", map[string]any{"muted": true}},
			{"mac.set_output_device", map[string]any{"device": "fixture"}},
			{"mac.set_appearance", map[string]any{"appearance": "dark"}},
			{"mac.launch_app", map[string]any{"bundle_id": "com.apple.calculator"}},
			{"mac.activate_app", map[string]any{"bundle_id": "com.apple.calculator"}},
			{"mac.quit_app", map[string]any{"bundle_id": "com.apple.calculator"}},
			{"mac.keep_awake", map[string]any{"on": true, "duration": 60}},
			{"mac.keep_awake", map[string]any{"on": false}},
		} {
			raw, _ := json.Marshal(test.args)
			var result contracts.CommandRecord
			err = client.Call(ctx, test.op, contracts.Command{ID: contracts.NewID("cmd"), Op: test.op, Args: raw, WaitMS: 15000}, &result)
			add(test.op+"_readback", err == nil && result.State == contracts.CommandObserved && contracts.MacCompletionEvidence(result.Evidence, result.RequestHash))
		}
		macClipboardCommands(ctx, client, w, false, add)
		return
	}
	if level == "real" {
		var out contracts.ReadResult
		err = w.Await(20*time.Second, func() bool {
			return client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "mac.power", Limit: 100}, &out) == nil && len(out.Records) > 0
		})
		add("power_captured", err == nil)
		if err != nil {
			return
		}
		if s.ID == "MAC-POWER-REAL" {
			bytes, err := exec.CommandContext(ctx, "pmset", "-g", "batt").Output()
			match := regexp.MustCompile(`(\d+)%;`).FindSubmatch(bytes)
			add("pmset_battery_available", err == nil && len(match) == 2)
			if len(match) != 2 {
				return
			}
			expected, _ := strconv.ParseFloat(string(match[1]), 64)
			var power struct {
				Battery  float64 `json:"battery_percent"`
				Charging bool    `json:"charging"`
				Source   string  `json:"source"`
			}
			err = json.Unmarshal(out.Records[0].Data, &power)
			add("battery_matches_pmset", err == nil && power.Battery >= expected && power.Battery < expected+1)
			add("charging_matches_pmset", power.Charging == strings.Contains(string(bytes), "; charging;"))
			add("source_matches_pmset", strings.Contains(string(bytes), "AC Power") == strings.Contains(power.Source, "AC"))
			return
		}
		pid, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
		add("helper_pid", err == nil)
		if err != nil {
			return
		}
		cpu := func() float64 {
			b, err := exec.CommandContext(ctx, "ps", "-o", "time=", "-p", strings.TrimSpace(string(pid))).Output()
			if err != nil {
				return -1
			}
			parts := strings.Split(strings.TrimSpace(string(b)), ":")
			if len(parts) != 2 {
				return -1
			}
			m, _ := strconv.ParseFloat(parts[0], 64)
			v, _ := strconv.ParseFloat(parts[1], 64)
			return m*60 + v
		}
		before := cpu()
		started := time.Now()
		timer := time.NewTimer(5 * time.Minute)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			add("idle_completed", false)
			return
		case <-timer.C:
		}
		after := cpu()
		e.Metrics = map[string]any{"helper_cpu_percent": 100 * (after - before) / time.Since(started).Seconds(), "idle_seconds": time.Since(started).Seconds()}
		add("helper_cpu_under_two_percent", before >= 0 && after >= before && (after-before)/time.Since(started).Seconds() < 0.02)
		return
	}

	for _, collection := range []string{"mac.audio_device", "mac.display", "mac.power", "mac.network", "mac.app"} {
		for _, value := range []string{"first", "changed"} {
			start := time.Now()
			err = w.ControlAction("/mac", map[string]any{"collection": collection, "key": "fixture", "data": macFixture(collection, value)})
			add("fake_change_accepted", err == nil)
			err = w.Await(time.Second, func() bool {
				var out contracts.ReadResult
				if client.Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, Limit: 100}, &out) != nil {
					return false
				}
				for _, r := range out.Records {
					var d map[string]any
					json.Unmarshal(r.Data, &d)
					field := "name"
					switch collection {
					case "mac.display":
						field = "appearance"
					case "mac.power":
						field = "source"
					case "mac.network":
						field = "interface"
					}
					if d[field] == value && len(contracts.CheckShape(collection, r.Data)) == 0 {
						return true
					}
				}
				return false
			})
			add("change_visible_under_second", err == nil && time.Since(start) < time.Second)
		}
	}
	if s.ID == "MAC-IDLE" {
		var before, after contracts.ReadResult
		err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "mac.power", Limit: 100}, &before)
		timer := time.NewTimer(time.Second)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			add("idle_completed", false)
			return
		case <-timer.C:
		}
		nextErr := client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "mac.power", Limit: 100}, &after)
		a, _ := json.Marshal(before.Records)
		b, _ := json.Marshal(after.Records)
		add("idle_records_unchanged", err == nil && nextErr == nil && string(a) == string(b) && len(before.Records) > 0)
	}
	return
}

func macFixture(collection, value string) map[string]any {
	switch collection {
	case "mac.audio_device":
		return map[string]any{"name": value, "input": true, "output": true, "default": true, "volume": 0.5, "muted": value == "changed"}
	case "mac.display":
		return map[string]any{"appearance": value, "displays": []any{}}
	case "mac.power":
		return map[string]any{"source": value, "charging": true, "low_power_mode": false, "keep_awake_held": false, "battery_percent": 75}
	case "mac.network":
		return map[string]any{"interface": value, "connected": true, "vpn": false, "wifi_power": true}
	default:
		return map[string]any{"bundle_id": "com.example.bridge", "name": value, "running": true, "frontmost": true, "hidden": false}
	}
}
