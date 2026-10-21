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

func runRemindersStore(ctx context.Context, s Scenario, level string) Evidence {
	if level != "sim" {
		return runRemindersStoreReal(ctx, s, level)
	}
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	w, err := world.New(ctx, level, 1, 0)
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	report := filepath.Join(w.Dir, "reminders-store-fixture.json")
	arguments := []string{"--report", report}
	if s.ID == "REM-STORE-DECODE" {
		arguments = append(arguments, "--blob-checks")
	}
	output, err := exec.CommandContext(ctx, "packaging/dev/check-reminders-store.sh", arguments...).CombinedOutput()
	add("synthetic_sqlite_arbitrary_entities_wal_isolation_and_projection", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = string(output)
		return e
	}
	bytes, err := os.ReadFile(report)
	var fixture struct {
		Observations []contracts.Observation `json:"observations"`
	}
	add("synthetic_reader_observations", err == nil && json.Unmarshal(bytes, &fixture) == nil && len(fixture.Observations) == 3)
	if len(fixture.Observations) != 3 {
		return e
	}
	for _, observation := range fixture.Observations {
		err = w.ControlAction("/external-change", map[string]any{"collection": observation.Collection, "key": observation.NativeKey, "data": observation.Data})
		if err != nil {
			add("reader_capture_committed", false)
			return e
		}
	}
	add("store_fields_truth_gateway_and_replica", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	records, err := w.Records(w.Gateway)
	add("gateway_records", err == nil)
	var parent, child contracts.Record
	for _, record := range records {
		switch recordTitle(record) {
		case "Bridge Test Parent":
			parent = record
		case "Bridge Test Child":
			child = record
		case "Bridge Test List":
			d := rmData(record)
			add("list_native_emblem_pin_order_grocery_sharing", d["badge_emblem"] == "star" && d["pinned"] == true && d["display_order"] == float64(12) && d["categorize_groceries"] == true && d["shared_owner_name"] == "Bridge Test Owner")
		}
	}
	add("flagged_exact_and_parent_linked", parent.ID != "" && child.ID != "" && rmData(parent)["flagged"] == true && rmData(child)["parent_id"] == parent.ID)
	add("unsupported_order_and_sections_not_guessed", rmData(parent)["manual_order"] == nil && rmData(parent)["section_id"] == nil)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
