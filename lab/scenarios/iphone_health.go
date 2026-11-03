package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runIPhoneHealth(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "health_permission", Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
		return e
	}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-health-")
	add("isolated_home", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "proof")
	cmd := iphoneProjectCommand(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/HealthRead/HealthCapture.swift", "lab/ios/HealthProof.swift", "-o", binary)
	buildOutput, buildErr := cmd.CombinedOutput()
	err = buildErr
	if err != nil {
		fmt.Fprintf(os.Stderr, "Health proof compile: %s\n", buildOutput)
	}
	add("swift_capture_build", err == nil)
	if err == nil {
		output, runErr := exec.CommandContext(ctx, binary).Output()
		add("anchored_retry_restart_deletion", runErr == nil && strings.TrimSpace(string(output)) == "PASS")
	}
	gateway, phone, token, err := iphoneBinaryFixture(ctx, dir)
	add("binary_gateway_fixture", err == nil)
	if err != nil {
		return e
	}
	defer gateway.Close()
	healthToken, err := gateway.Command("token", "create", "--scope", "health:read")
	add("health_read_scope", err == nil)
	readSummary := func(period, date string) (contracts.HealthSummary, error) {
		var summary contracts.HealthSummary
		err := gateway.Invoke("health.summary", contracts.HealthSummaryQuery{Period: period, Date: date}, healthToken, &summary)
		return summary, err
	}
	value := 120.0
	start := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	sample := contracts.HealthSample{ID: "ab-test-steps", Type: "steps", Start: start, End: start.Add(time.Hour), Value: &value, Unit: "count"}
	batch := contracts.HealthBatch{Stream: "steps", Sequence: 1, Samples: []contracts.HealthSample{sample}, Deleted: []string{}}
	upload := func(b contracts.HealthBatch) int {
		raw, _ := json.Marshal(b)
		response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/health", raw)
		if err != nil {
			return 0
		}
		defer response.Body.Close()
		return response.StatusCode
	}
	add("signed_capture", upload(batch) == 200)
	add("retry_deduplicated", upload(batch) == 200)
	changed := batch
	changed.Samples = slices.Clone(batch.Samples)
	changed.Samples[0].Unit = "m"
	add("altered_retry_rejected", upload(changed) == 400)
	batch.Sequence = 3
	add("sequence_gap_rejected", upload(batch) == 400)
	batch.Sequence = 1
	var result contracts.ReadResult
	err = gateway.Invoke("records.query", contracts.ReadQuery{Collection: "health.sample"}, healthToken, &result)
	add("sample_record", err == nil && len(result.Records) == 1 && result.Records[0].Version == 1)
	input := json.RawMessage(`{"period":"day","date":"2026-10-28"}`)
	var unauthorized contracts.HealthSummary
	err = gateway.Invoke("health.summary", input, token, &unauthorized)
	add("summary_scope_denied", err != nil)
	summary, err := readSummary("day", "2026-10-28")
	add("day_summary", err == nil && summary.Metrics["steps/count"].Sum == 120)
	summary, err = readSummary("week", "2026-10-29")
	add("week_summary", err == nil && summary.Start.Day() == 26 && summary.Metrics["steps/count"].Count == 1)
	stage := 3
	sleepStart := time.Date(2026, 10, 27, 23, 0, 0, 0, time.UTC)
	sleep := contracts.HealthBatch{Stream: "sleep", Sequence: 1, Samples: []contracts.HealthSample{{ID: "ab-test-sleep", Type: "sleep", Start: sleepStart, End: sleepStart.Add(8 * time.Hour), Category: &stage}}, Deleted: []string{}}
	add("sleep_stage_capture", upload(sleep) == 200)
	summary, err = readSummary("day", "2026-10-28")
	add("sleep_stage_clipped_to_day", err == nil && summary.Metrics["sleep//3"].Seconds == 7*3600)
	sleep.Sequence = 2
	sleep.Samples = []contracts.HealthSample{}
	sleep.Deleted = []string{"ab-test-sleep"}
	add("sleep_deletion", upload(sleep) == 200)
	batch = contracts.HealthBatch{Stream: "steps", Sequence: 2, Samples: []contracts.HealthSample{}, Deleted: []string{sample.ID}}
	add("signed_deletion", upload(batch) == 200)
	err = gateway.Invoke("records.query", contracts.ReadQuery{Collection: "health.sample", IncludeDeleted: true}, healthToken, &result)
	add("deletion_record", err == nil && len(result.Records) == 2 && result.Records[0].Deleted && result.Records[0].Version == 2 && result.Records[1].Deleted && result.Records[1].Version == 2)
	summary, err = readSummary("day", "2026-10-28")
	add("deleted_excluded", err == nil && len(summary.Metrics) == 0)
	workouts := contracts.HealthBatch{Stream: "workouts", Sequence: 1, Samples: []contracts.HealthSample{
		{ID: "ab-test-workout-1", Type: "workouts", Start: start, End: start.Add(time.Hour)},
		{ID: "ab-test-workout-2", Type: "workouts", Start: start.Add(2 * time.Hour), End: start.Add(3 * time.Hour)},
	}, Deleted: []string{}}
	add("workouts_capture", upload(workouts) == 200)
	for _, period := range []string{"day", "week"} {
		summary, err = readSummary(period, "2026-10-28")
		add(period+"_unitless_workouts_accumulate", err == nil && summary.Metrics["workouts"].Count == 2 && summary.Metrics["workouts"].Seconds == 7200)
	}
	raw, _ := json.Marshal(result.Records)
	digest := sha256.Sum256(raw)
	e.Assertions = append(e.Assertions, Assertion{Name: "digest", Passed: true, Detail: hex.EncodeToString(digest[:])})
	return e
}
