package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRemindersStoreReal(ctx context.Context, s Scenario, level string) (result Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	tag := "[ab-test " + e.RunID + "]"
	listName := "Bridge Test " + tag
	listID, itemID := "", ""
	created, cleaned := false, false
	setup := world.NativeSetup{
		BeforeStart: func(w *world.World) error {
			list, err := w.Truth("fixture", map[string]any{"source": "reminders", "action": "create_list", "list": listName, "tag": tag})
			if err != nil {
				return err
			}
			listID, _ = list["id"].(string)
			created, _ = list["created"].(bool)
			if !created || listID == "" {
				return errors.New("unique_fixture_list_not_created")
			}
			item, err := w.Truth("fixture", map[string]any{"source": "reminders", "action": "create", "list": listName, "list_id": listID, "tag": tag, "title": tag + " identity"})
			if err != nil {
				return err
			}
			itemID, _ = item["id"].(string)
			if itemID == "" {
				return errors.New("fixture_item_identifier_missing")
			}
			return nil
		},
		Cleanup: func(w *world.World) error {
			if !created {
				return nil
			}
			request := map[string]any{"source": "reminders", "action": "cleanup", "list": listName, "list_id": listID, "tag": tag}
			if _, err := w.RecoverTruth("fixture", request); err != nil {
				return err
			}
			request["action"] = "cleanup_list"
			if _, err := w.RecoverTruth("fixture", request); err != nil {
				return err
			}
			sweep, err := w.RecoverTruth("observe", map[string]any{"source": "reminders", "list": listName, "list_id": listID, "tag": tag})
			cleaned = err == nil && sweep["list_exists"] == false
			if !cleaned {
				return errors.New("fixture_cleanup_sweep_failed")
			}
			return nil
		},
	}
	w, err := world.NewRealWithSetup(ctx, level, 0, setup)
	add("tagged_eventkit_fixture_and_private_signed_apps", err == nil)
	if err != nil {
		return e
	}
	defer func() {
		w.Close()
		result.Assertions = append(result.Assertions, Assertion{Name: "created_fixture_cleanup_and_sweep", Passed: cleaned})
		if !cleaned || len(w.TruthFailures) != 0 {
			result.Passed = false
		}
	}()
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "w5-store-"+e.RunID+".json")
	defer os.Remove(report)
	probeCtx, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	output, err := exec.CommandContext(probeCtx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "reminders-store-proof", "--tag", tag, "--item-id", itemID, "--list-id", listID, "--report", report).CombinedOutput()
	add("helper_probe_launched", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = string(output)
		return e
	}
	bytes, err := os.ReadFile(report)
	var proof struct {
		DAEqual        bool           `json:"da_equals_eventkit"`
		UUIDEqual      bool           `json:"uuid_equals_eventkit"`
		ExternalEqual  bool           `json:"external_equals_eventkit"`
		ListEqual      bool           `json:"list_uuid_equals_eventkit"`
		ReminderFields map[string]any `json:"reminder_fields"`
		ListFields     map[string]any `json:"list_fields"`
		ReadMS         float64        `json:"store_read_ms"`
	}
	decoded := err == nil && json.Unmarshal(bytes, &proof) == nil
	add("helper_store_consistent_copy_report_decoded", decoded)
	if !decoded {
		return e
	}
	add("eventkit_calendar_item_identifier_equals_da_text", proof.DAEqual)
	add("external_identifier_is_not_identity_fallback", !proof.ExternalEqual)
	add("list_uuid_bytes_link_to_eventkit", proof.ListEqual)
	add("native_flag_and_subtask_absence_exact", proof.ReminderFields["flagged"] == false && proof.ReminderFields["parent_id"] == nil)
	e.Metrics = map[string]any{"identity_da_equal": proof.DAEqual, "identity_uuid_equal": proof.UUIDEqual, "identity_external_equal": proof.ExternalEqual, "store_read_single_ms": proof.ReadMS}
	e.Assertions = append(e.Assertions, Assertion{Name: "appletruth_store_counts", Status: "limited", Detail: "AppleTruth lacks Full Disk Access to the Reminders Core Data store"})
	e.Passed = true
	for _, assertion := range e.Assertions {
		if assertion.Status != "limited" {
			e.Passed = e.Passed && assertion.Passed
		}
	}
	e.Status = "limited"
	return e
}
