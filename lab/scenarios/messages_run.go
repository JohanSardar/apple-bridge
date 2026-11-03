package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func messageNormalize(records []contracts.Record) map[string]any {
	names := map[string]string{}
	for _, r := range records {
		var data map[string]any
		json.Unmarshal(r.Data, &data)
		title, _ := data["title"].(string)
		names[r.ID] = title
	}
	var translate func(any) any
	translate = func(value any) any {
		switch v := value.(type) {
		case map[string]any:
			if ref, ok := v["$ref"].(map[string]any); ok {
				key, _ := ref["native_key"].(string)
				return names[key]
			}
			out := map[string]any{}
			for k, child := range v {
				out[k] = translate(child)
			}
			return out
		case []any:
			out := make([]any, len(v))
			for i, child := range v {
				out[i] = translate(child)
			}
			return out
		case string:
			if title, ok := names[v]; ok {
				return title
			}
			return v
		default:
			return v
		}
	}
	out := map[string]any{}
	for _, r := range records {
		if r.Source == contracts.System || strings.HasPrefix(r.Collection, "system.") || r.Deleted {
			continue
		}
		var data map[string]any
		json.Unmarshal(r.Data, &data)
		out[r.Collection+"/"+recordTitle(r)] = translate(data)
	}
	return out
}

func messagesConverged(w *world.World) bool {
	truth, err := w.Inspect()
	if err != nil {
		return false
	}
	expected := []contracts.Record{}
	for _, o := range truth {
		if o.Kind != "absent" {
			expected = append(expected, contracts.Record{ID: o.NativeKey, Collection: o.Collection, Source: contracts.CollectionSource(o.Collection), Data: o.Data})
		}
	}
	gateway, err := w.Records(w.Gateway)
	if err != nil {
		return false
	}
	if !reflect.DeepEqual(messageNormalize(expected), messageNormalize(gateway)) {
		return false
	}
	cursor, err := w.Cursor(w.Gateway)
	if err != nil {
		return false
	}
	for _, endpoint := range w.Replicas {
		current, err := w.Cursor(endpoint)
		if err != nil || current != cursor {
			return false
		}
		records, err := w.Records(endpoint)
		if err != nil || !reflect.DeepEqual(gateway, records) {
			return false
		}
	}
	return true
}

func runMessages(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	evidence := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: ok})
	}
	seed, replicas, valid := messageScenarioOptions(s)
	if !valid {
		add("valid_messages_scenario", false)
		return evidence
	}
	if level != "sim" {
		return runRealMessages(ctx, s, level, evidence)
	}
	return runMessagesSim(ctx, s, level, evidence, seed, replicas)
}

func messageGeneration(w *world.World) string {
	response, err := w.Control.Get("http://sim/messages-status")
	if err != nil {
		return ""
	}
	defer response.Body.Close()
	var status struct {
		Generation string `json:"generation"`
	}
	json.NewDecoder(response.Body).Decode(&status)
	return status.Generation
}
func sameMessageIDs(a, b []contracts.Record) bool {
	ids := func(records []contracts.Record) map[string]string {
		out := map[string]string{}
		for _, record := range records {
			if strings.HasPrefix(record.Collection, "messages.") {
				out[key(record)] = record.ID
			}
		}
		return out
	}
	return reflect.DeepEqual(ids(a), ids(b))
}

func messageSeen(observer *watcher, title string) (Seen, bool) {
	observer.mu.Lock()
	defer observer.mu.Unlock()
	for _, result := range observer.seen {
		if result.Change.Record.Collection == "messages.message" && recordTitle(result.Change.Record) == title {
			return result, true
		}
	}
	return Seen{}, false
}

func messageScenarioOptions(s Scenario) (int, int, bool) {
	seed, replicas := 256, 1
	if len(s.Given) != 2 || len(s.When) != 1 || len(s.Then) != 1 || len(s.Cleanup) != 1 {
		return 0, 0, false
	}
	for _, step := range s.Given {
		if len(step) != 1 {
			return 0, 0, false
		}
		for name, options := range step {
			switch name {
			case "world":
				if len(options) != 1 {
					return 0, 0, false
				}
				replicas = number(options, "replicas", -1)
			case "messages":
				if len(options) != 1 {
					return 0, 0, false
				}
				seed = number(options, "count", -1)
			default:
				return 0, 0, false
			}
		}
	}
	if seed < 2 || seed > 20000 || replicas != 1 {
		return 0, 0, false
	}
	if len(s.When[0]) != 1 || len(s.Then[0]) != 1 || len(s.Cleanup[0]) != 1 {
		return 0, 0, false
	}
	expected := strings.ToLower(strings.TrimPrefix(s.ID, "MSG-"))
	for name, options := range s.When[0] {
		if name != "messages_read" || len(options) != 1 || options["scenario"] != expected {
			return 0, 0, false
		}
	}
	for name, options := range s.Then[0] {
		if name != "messages_truth" || len(options) != 0 {
			return 0, 0, false
		}
	}
	for name, options := range s.Cleanup[0] {
		if name != "world" || len(options) != 0 {
			return 0, 0, false
		}
	}
	return seed, replicas, true
}

func messageConvergenceDetails(w *world.World) {
	truth, _ := w.Inspect()
	expected := []contracts.Record{}
	for _, o := range truth {
		if o.Kind != "absent" {
			expected = append(expected, contracts.Record{ID: o.NativeKey, Collection: o.Collection, Source: contracts.CollectionSource(o.Collection), Data: o.Data})
		}
	}
	gateway, err := w.Records(w.Gateway)
	counts := func(records []contracts.Record) map[string]int {
		out := map[string]int{}
		for _, r := range records {
			out[r.Collection]++
		}
		return out
	}
	states := []map[string]any{}
	for _, r := range gateway {
		if r.Collection == "system.source" {
			var d map[string]any
			json.Unmarshal(r.Data, &d)
			states = append(states, map[string]any{"source": d["source"], "state": d["state"], "reason": d["reason"]})
		}
	}
	a, b := messageNormalize(expected), messageNormalize(gateway)
	missing := 0
	fields := map[string]int{}
	for key, left := range a {
		right, ok := b[key]
		if !ok {
			missing++
			continue
		}
		if reflect.DeepEqual(left, right) {
			continue
		}
		x, _ := left.(map[string]any)
		y, _ := right.(map[string]any)
		for field, value := range x {
			if !reflect.DeepEqual(value, y[field]) {
				fields[field]++
			}
		}
	}
	report := map[string]any{"expected": counts(expected), "gateway": counts(gateway), "read_error": err != nil, "missing": missing, "different_fields": fields, "states": states}
	encoded, _ := json.Marshal(report)
	fmt.Fprintln(os.Stderr, "message_projection_diagnostic", string(encoded))
}

func messagesHistorySeen(observer *watcher, expected int) (time.Time, bool) {
	observer.mu.Lock()
	defer observer.mu.Unlock()
	count := 0
	var last time.Time
	for _, seen := range observer.seen {
		if seen.Change.Record.Collection == "messages.message" && !seen.Change.Record.Deleted {
			count++
			if seen.At.After(last) {
				last = seen.At
			}
		}
	}
	return last, count >= expected
}

func messagesCaptureStats(w *world.World) map[string]float64 {
	out := map[string]float64{}
	response, err := w.Control.Get("http://lab/messages-status")
	if err != nil {
		return out
	}
	defer response.Body.Close()
	var data map[string]any
	if json.NewDecoder(response.Body).Decode(&data) == nil {
		for key, value := range data {
			if number, ok := value.(float64); ok {
				out[key] = number
			}
		}
	}
	return out
}

func messagesHostLoad() map[string]any {
	load, _ := exec.Command("sysctl", "-n", "vm.loadavg").Output()
	fields := strings.Fields(strings.Trim(string(load), "{} \n"))
	values := []float64{}
	for _, field := range fields {
		if number, err := strconv.ParseFloat(field, 64); err == nil {
			values = append(values, number)
		}
	}
	slots, _ := filepath.Glob("/tmp/apple-bridge-heavy-*.lock/pid")
	return map[string]any{"load_average": values, "logical_cpus": runtime.NumCPU(), "gomaxprocs": runtime.GOMAXPROCS(0), "occupied_heavy_slots": len(slots)}
}
