package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func runIPhoneHealth(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "health_permission", Detail: "limited: needs Allow tap"}}
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
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/HealthRead/HealthCapture.swift", "lab/ios/HealthProof.swift", "-o", binary)
	cmd.Env = append(os.Environ(), "SDKROOT=")
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
	store, err := state.Open(filepath.Join(dir, "state"))
	add("state", err == nil)
	if err != nil {
		return e
	}
	defer store.Close()
	authority, err := trust.Open(filepath.Join(dir, "trust"))
	add("authority", err == nil)
	if err != nil {
		return e
	}
	delivery := iphone.NewDelivery(store, authority)
	mux := http.NewServeMux()
	mux.Handle("/v1/iphone/pair", authority.IPhoneHandler())
	mux.Handle("/v1/iphone/", delivery.Handler())
	server := httptest.NewTLSServer(mux)
	defer server.Close()
	code, err := authority.PairIPhoneCode(server.URL, false, time.Minute)
	add("pair_code", err == nil)
	phone, err := simapple.NewIPhone()
	add("phone", err == nil)
	if err != nil {
		return e
	}
	status, err := phone.Pair(server.Client(), server.URL, code)
	add("paired", err == nil && status == 200)
	value := 120.0
	start := time.Date(2026, 10, 28, 12, 0, 0, 0, time.UTC)
	sample := contracts.HealthSample{ID: "ab-test-steps", Type: "steps", Start: start, End: start.Add(time.Hour), Value: &value, Unit: "count"}
	batch := contracts.HealthBatch{Stream: "steps", Sequence: 1, Samples: []contracts.HealthSample{sample}, Deleted: []string{}}
	upload := func(b contracts.HealthBatch) int {
		raw, _ := json.Marshal(b)
		response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/health", raw)
		if err != nil {
			return 0
		}
		defer response.Body.Close()
		return response.StatusCode
	}
	add("signed_capture", upload(batch) == 200)
	add("retry_deduplicated", upload(batch) == 200)
	changed := batch
	changed.Samples = append([]contracts.HealthSample(nil), batch.Samples...)
	changed.Samples[0].Unit = "m"
	add("altered_retry_rejected", upload(changed) == 400)
	batch.Sequence = 3
	add("sequence_gap_rejected", upload(batch) == 400)
	batch.Sequence = 1
	result, err := store.Read(ctx, trust.Visibility{"health": true}, contracts.ReadQuery{Collection: "health.sample"})
	add("sample_record", err == nil && len(result.Records) == 1 && result.Records[0].Version == 1)
	service := operations.Service{State: store}
	input := json.RawMessage(`{"period":"day","date":"2026-10-28"}`)
	_, err = service.Invoke(ctx, trust.Principal{}, "health.summary", input)
	add("summary_scope_denied", err != nil)
	principal := trust.Principal{Grants: []contracts.Grant{"health:read"}}
	rawSummary, err := service.Invoke(ctx, principal, "health.summary", input)
	summary, ok := rawSummary.(contracts.HealthSummary)
	add("day_summary", err == nil && ok && summary.Metrics["steps/count"].Sum == 120)
	summary, err = store.HealthSummary(ctx, principal.Visibility(), contracts.HealthSummaryQuery{Period: "week", Date: "2026-10-29"})
	add("week_summary", err == nil && summary.Start.Day() == 26 && summary.Metrics["steps/count"].Count == 1)
	stage := 3
	sleepStart := time.Date(2026, 10, 27, 23, 0, 0, 0, time.UTC)
	sleep := contracts.HealthBatch{Stream: "sleep", Sequence: 1, Samples: []contracts.HealthSample{{ID: "ab-test-sleep", Type: "sleep", Start: sleepStart, End: sleepStart.Add(8 * time.Hour), Category: &stage}}, Deleted: []string{}}
	add("sleep_stage_capture", upload(sleep) == 200)
	summary, err = store.HealthSummary(ctx, principal.Visibility(), contracts.HealthSummaryQuery{Period: "day", Date: "2026-10-28"})
	add("sleep_stage_clipped_to_day", err == nil && summary.Metrics["sleep//3"].Seconds == 7*3600)
	sleep.Sequence = 2
	sleep.Samples = []contracts.HealthSample{}
	sleep.Deleted = []string{"ab-test-sleep"}
	add("sleep_deletion", upload(sleep) == 200)
	batch = contracts.HealthBatch{Stream: "steps", Sequence: 2, Samples: []contracts.HealthSample{}, Deleted: []string{sample.ID}}
	add("signed_deletion", upload(batch) == 200)
	result, err = store.Read(ctx, principal.Visibility(), contracts.ReadQuery{Collection: "health.sample", IncludeDeleted: true})
	add("deletion_record", err == nil && len(result.Records) == 2 && result.Records[0].Deleted && result.Records[0].Version == 2 && result.Records[1].Deleted && result.Records[1].Version == 2)
	summary, err = store.HealthSummary(ctx, principal.Visibility(), contracts.HealthSummaryQuery{Period: "day", Date: "2026-10-28"})
	add("deleted_excluded", err == nil && len(summary.Metrics) == 0)
	workouts := contracts.HealthBatch{Stream: "workouts", Sequence: 1, Samples: []contracts.HealthSample{
		{ID: "ab-test-workout-1", Type: "workouts", Start: start, End: start.Add(time.Hour)},
		{ID: "ab-test-workout-2", Type: "workouts", Start: start.Add(2 * time.Hour), End: start.Add(3 * time.Hour)},
	}, Deleted: []string{}}
	add("workouts_capture", upload(workouts) == 200)
	for _, period := range []string{"day", "week"} {
		summary, err = store.HealthSummary(ctx, principal.Visibility(), contracts.HealthSummaryQuery{Period: period, Date: "2026-10-28"})
		add(period+"_unitless_workouts_accumulate", err == nil && summary.Metrics["workouts"].Count == 2 && summary.Metrics["workouts"].Seconds == 7200)
	}
	raw, _ := json.Marshal(result.Records)
	digest := sha256.Sum256(raw)
	e.Assertions = append(e.Assertions, Assertion{Name: "digest", Passed: true, Detail: hex.EncodeToString(digest[:])})
	return e
}
