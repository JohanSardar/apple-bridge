package scenarios

import (
	"bytes"
	"context"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runHelperSoak(ctx context.Context, w *world.World, e Evidence) (result Evidence) {
	tag := "[ab-test " + contracts.NewID("soak") + "]"
	defer func() {
		_, err := w.RecoverTruth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": tag})
		if err != nil {
			result.Passed = false
			result.Assertions = append(result.Assertions, Assertion{Name: "soak_cleanup", Passed: false})
		}
	}()
	pidPath := filepath.Join(w.Gateway.Home, "helper", "helper.pid")
	before, err := os.ReadFile(pidPath)
	if err != nil {
		return e
	}
	reports := filepath.Join(os.Getenv("HOME"), "Library", "Logs", "DiagnosticReports")
	initial, err := os.ReadDir(reports)
	if err != nil {
		return e
	}
	existing := map[string]bool{}
	for _, report := range initial {
		existing[report.Name()] = true
	}
	started := time.Now()
	nextFixture := started
	stable := true
	for time.Since(started) < 10*time.Minute {
		current, readErr := os.ReadFile(pidPath)
		if readErr != nil || !bytes.Equal(before, current) {
			stable = false
			break
		}
		if !time.Now().Before(nextFixture) {
			_, err = w.Truth("fixture", map[string]any{"action": "create", "list": "Bridge Test", "tag": tag, "title": tag + contracts.NewID("deadline")})
			if err != nil {
				stable = false
				break
			}
			nextFixture = time.Now().Add(2 * time.Minute)
		}
		if sleep(ctx, 500*time.Millisecond) != nil {
			stable = false
			break
		}
	}
	noCrash := true
	final, err := os.ReadDir(reports)
	if err != nil {
		noCrash = false
	}
	for _, report := range final {
		if existing[report.Name()] || !strings.HasPrefix(report.Name(), "AppleBridgeHelper") {
			continue
		}
		data, readErr := os.ReadFile(filepath.Join(reports, report.Name()))
		if readErr != nil || bytes.Contains(data, []byte(w.HelperApp)) {
			noCrash = false
		}
	}
	ticks := len(readNativeTrace(filepath.Join(w.Gateway.Home, "helper", "session-timers.jsonl")))
	deadlines := 0
	for _, row := range readNativeTrace(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl")) {
		if row.Stage == "fetch_timer_fired" {
			deadlines++
		}
	}
	elapsed := float64(time.Since(started)) / float64(time.Millisecond)
	e.Latency["continuous_helper_session_duration"] = stats([]float64{elapsed}, 0)
	e.Assertions = append(e.Assertions, Assertion{Name: "same_helper_pid_ten_minutes", Passed: stable && elapsed >= 600000}, Assertion{Name: "no_new_world_helper_crash_reports", Passed: noCrash}, Assertion{Name: "six_hundred_lifetime_timer_deadlines", Passed: ticks >= 600}, Assertion{Name: "multiple_fetch_timer_deadlines", Passed: deadlines >= 4})
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}

func runBackfillPriority(ctx context.Context, s Scenario, level string, e Evidence) (result Evidence) {
	tag := "[ab-test " + contracts.NewID("bulk") + "]"
	original := tag + " original"
	updated := tag + " updated"
	setup := world.NativeSetup{HoldBackfill: true, BeforeStart: func(w *world.World) error {
		_, err := w.Truth("fixture", map[string]any{"action": "create", "list": "Bridge Test", "tag": tag, "title": original})
		return err
	}, Cleanup: func(w *world.World) error {
		_, err := w.RecoverTruth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": tag})
		return err
	}}
	w, err := world.NewRealWithSetup(ctx, level, 1, setup)
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "backfill_world", Passed: false})
		return e
	}
	defer func() {
		w.Close()
		if len(w.TruthFailures) > 0 {
			result.Passed = false
		}
	}()
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	held := filepath.Join(w.Gateway.Home, "helper", "lab-backfill-held")
	if w.Await(5*time.Second, func() bool { _, err := os.Stat(held); return err == nil }) != nil {
		add("initial_backfill_held", false)
		return e
	}
	add("initial_backfill_held", true)
	_, err = w.Truth("fixture", map[string]any{"action": "modify", "list": "Bridge Test", "tag": tag, "title": original, "new_title": updated})
	add("external_edit_during_backfill", err == nil)
	visible := func() bool {
		for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			records, err := w.Records(endpoint)
			if err != nil {
				return false
			}
			count := 0
			for _, record := range records {
				title := recordTitle(record)
				if title == original {
					return false
				}
				if title == updated {
					count++
				}
			}
			if count != 1 {
				return false
			}
		}
		return true
	}
	add("live_visible_before_initial_backfill_commit", w.Await(5*time.Second, visible) == nil)
	before := readNativeTrace(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"))
	heldCaptureAbsent := true
	for _, row := range before {
		if row.Stage == "capture_sent" && !row.Live {
			heldCaptureAbsent = false
		}
	}
	checkpoint := filepath.Join(w.Gateway.Home, "helper", "lab-committed-reminders")
	prior, _ := os.ReadFile(checkpoint)
	os.Remove(filepath.Join(w.Gateway.Home, "helper", "lab-hold-backfill"))
	add("backfill_committed_after_live", w.Await(10*time.Second, func() bool {
		current, err := os.ReadFile(checkpoint)
		return err == nil && !bytes.Equal(current, prior)
	}) == nil)
	add("no_bulk_capture_before_live", heldCaptureAbsent)
	add("backfill_does_not_overwrite_live", visible())
	truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag, "title": updated})
	records, _ := truth["records"].([]any)
	add("independent_updated_truth", err == nil && len(records) == 1)
	add("stable_record_cursor_convergence", w.Await(5*time.Second, func() bool {
		records, head, err := w.RecordsAt(w.Gateway)
		if err != nil {
			return false
		}
		for _, endpoint := range w.Replicas {
			replica, cursor, err := w.RecordsAt(endpoint)
			if err != nil || cursor != head || !reflect.DeepEqual(records, replica) {
				return false
			}
		}
		return true
	}) == nil)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
