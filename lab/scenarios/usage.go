package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"time"
)

func usageRecords(ctx context.Context, endpoint world.Endpoint, protocol string) (contracts.ReadResult, error) {
	result := contracts.ReadResult{}
	for _, collection := range []string{"usage.device", "usage.app_day"} {
		after := ""
		for {
			var page contracts.ReadResult
			err := (scripted.Client{Endpoint: endpoint, Protocol: protocol}).Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, Limit: 1000, After: after, IncludeAliases: true}, &page)
			if err != nil {
				return result, err
			}
			result.Records = append(result.Records, page.Records...)
			result.Aliases = append(result.Aliases, page.Aliases...)
			if page.Next == "" {
				break
			}
			after = page.Next
		}
	}
	return result, nil
}
func usageRows(result contracts.ReadResult) map[string]map[string]float64 {
	aliases := map[string]string{}
	for _, alias := range result.Aliases {
		if alias.Collection == "usage.app_day" {
			sum := sha256.Sum256([]byte(alias.NativeKey))
			aliases[alias.RecordID] = hex.EncodeToString(sum[:])
		}
	}
	rows := map[string]map[string]float64{}
	for _, record := range result.Records {
		if record.Collection != "usage.app_day" {
			continue
		}
		var value map[string]any
		json.Unmarshal(record.Data, &value)
		key := aliases[record.ID]
		if key == "" {
			continue
		}
		row := map[string]float64{}
		for _, field := range []string{"seconds_estimated", "clipped_seconds", "sessions"} {
			row[field], _ = value[field].(float64)
		}
		rows[key] = row
	}
	return rows
}
func runUsage(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	fail := func(name string) Evidence { add(name, false); return e }
	if s.ID == "USE-SEGB-DECODE" {
		output, err := os.ReadFile("native/.build/native-output")
		if err != nil {
			return fail("native_decoder_built")
		}
		cmd := exec.CommandContext(ctx, filepath.Join(strings.TrimSpace(string(output)), "UsageChecks"), "lab/fixtures/usage")
		err = cmd.Run()
		add("scrubbed_segb_and_day_math", err == nil)
		e.Passed = err == nil
		return e
	}
	w, err := world.NewUsage(ctx, level)
	if err != nil {
		return fail("world_start")
	}
	defer w.Close()
	if level == "real" {
		truth, err := w.Truth("observe", map[string]any{"source": "usage"})
		if err != nil {
			return fail("independent_usage_truth_access")
		}
		expected := map[string]map[string]float64{}
		raw, _ := json.Marshal(truth["rows"])
		if json.Unmarshal(raw, &expected) != nil || len(expected) == 0 {
			return fail("truth_has_usage")
		}
		matches := func() bool {
			actual, err := usageRecords(ctx, w.Gateway, "rest")
			if err != nil {
				return false
			}
			rows := usageRows(actual)
			if len(rows) != len(expected) {
				return false
			}
			for key, want := range expected {
				got, ok := rows[key]
				if !ok {
					return false
				}
				for field, value := range want {
					if math.Abs(got[field]-value) > 0.001 {
						return false
					}
				}
			}
			return true
		}
		add("independent_day_totals_match", w.Await(45*time.Second, matches) == nil)
	} else {
		base := float64(1800100800)
		event := func(offset float64, start bool, bundle string) map[string]any {
			return map[string]any{"at": base + offset, "starting": start, "bundle": bundle}
		}
		send := func(device, platform string, events ...map[string]any) error {
			return w.ControlAction("/usage", map[string]any{"device": device, "platform": platform, "events": events})
		}
		app := "test.synthetic.alpha"
		if send("local", "mac", event(0, true, app), event(600, true, "loginwindow"), event(3600, false, app), event(0, true, app), event(999, false, "")) != nil {
			return fail("usage_fixture")
		}
		totals := func(endpoint world.Endpoint, device string) (float64, float64, string) {
			result, err := usageRecords(ctx, endpoint, "rest")
			if err != nil {
				return -1, -1, ""
			}
			ids := map[string]string{}
			for _, alias := range result.Aliases {
				if alias.Collection == "usage.device" {
					ids[alias.RecordID] = alias.NativeKey
				}
			}
			seconds, clipped, latest := float64(0), float64(0), ""
			for _, record := range result.Records {
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				if record.Collection == "usage.device" && ids[record.ID] == device {
					latest, _ = data["latest_event_at"].(string)
				}
				if record.Collection == "usage.app_day" {
					id, _ := data["device_id"].(string)
					if ids[id] == device {
						s, _ := data["seconds_estimated"].(float64)
						c, _ := data["clipped_seconds"].(float64)
						seconds += s
						clipped += c
					}
				}
			}
			return seconds, clipped, latest
		}
		if w.Await(10*time.Second, func() bool { s, c, _ := totals(w.Gateway, "local"); return s == 600 && c == 3000 }) != nil {
			return fail("mac_lock_clipped")
		}
		add("mac_lock_clipped", true)
		switch s.ID {
		case "USE-IMPORT":
			add("remote_import", send("remote", "iphone", event(0, true, app), event(120, false, app)) == nil)
			add("per_device_totals", w.Await(10*time.Second, func() bool { s, _, _ := totals(w.Gateway, "remote"); return s == 120 }) == nil)
		case "USE-LOCK-CLIP":
			add("screensaver_batch", send("local", "mac", event(4000, true, app), event(4100, true, "com.apple.ScreenSaver.Engine"), event(4600, false, app)) == nil)
			add("screensaver_clipped", w.Await(10*time.Second, func() bool { s, c, _ := totals(w.Gateway, "local"); return s == 700 && c == 3500 }) == nil)
		case "USE-REMOTE-LAG":
			if send("remote", "iphone", event(-86400, true, app), event(-85800, false, app)) != nil {
				return fail("remote_initial_batch")
			}
			before, _, fresh := totals(w.Gateway, "remote")
			add("remote_older_freshness", before == 600 && fresh != "")
			started := time.Now()
			if send("remote", "iphone", event(-85000, true, app), event(-84700, false, app)) != nil {
				return fail("remote_delayed_batch")
			}
			add("past_day_revised", w.Await(10*time.Second, func() bool { s, _, next := totals(w.Gateway, "remote"); return s == 900 && next > fresh }) == nil)
			elapsed := float64(time.Since(started).Microseconds()) / 1000
			e.Latency["batch_to_gateway_ms"] = Latency{Samples: 1, P50: elapsed, P95: elapsed, Max: elapsed}
			add("batch_live_under_one_second", elapsed < 1000)
		case "USE-READONLY":
			for _, op := range []string{"create", "update", "delete", "reset", "complete"} {
				for _, protocol := range []string{"rest", "mcp"} {
					var response any
					err := (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "usage."+op, map[string]any{}, &response)
					add("usage_"+op+"_"+protocol+"_unsupported", errors.Is(err, contracts.ErrUnsupported))
				}
			}
		}
	}
	add("replica_converged", w.Await(30*time.Second, func() bool {
		gateway, ge := usageRecords(ctx, w.Gateway, "rest")
		replica, re := usageRecords(ctx, w.Replicas[0], "mcp")
		return ge == nil && re == nil && len(gateway.Records) > 0 && reflect.DeepEqual(recordMultiset(gateway.Records, true), recordMultiset(replica.Records, true))
	}) == nil)
	result, err := usageRecords(ctx, w.Gateway, "mcp")
	metadata := err == nil
	devices, days := 0, 0
	for _, record := range result.Records {
		var data map[string]any
		json.Unmarshal(record.Data, &data)
		if record.Collection == "usage.device" {
			devices++
			metadata = metadata && data["name"] != "" && data["platform"] != "" && data["latest_event_at"] != nil
		} else {
			days++
			metadata = metadata && data["provenance"] == "biome.app_infocus" && data["decoder_version"] == "infocus-segb-1" && data["uncertainty"] != ""
		}
	}
	add("provenance_and_device_freshness", metadata && devices > 0 && days > 0)
	e.Metrics = map[string]any{"devices": devices, "app_days": days}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
