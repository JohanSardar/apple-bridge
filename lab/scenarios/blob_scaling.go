package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type blobScalingSample struct {
	Rows                int            `json:"rows"`
	ProtectedReferences int            `json:"protected_references"`
	References          int            `json:"references"`
	CaptureMS           float64        `json:"capture_ms"`
	RowsPerSecond       float64        `json:"rows_per_second"`
	MSPerCapture        float64        `json:"ms_per_capture"`
	Before              map[string]any `json:"before"`
	After               map[string]any `json:"after"`
}

func blobHostTelemetry(ctx context.Context) map[string]any {
	out := messagesHostLoad()
	out["at"] = time.Now().UTC()
	queued, _ := filepath.Glob("/tmp/apple-bridge-heavy-queue/*")
	out["queued_heavy_commands"] = len(queued)
	data, err := exec.CommandContext(ctx, "top", "-l", "2", "-s", "1", "-n", "0").Output()
	out["cpu_sample_available"] = false
	if err == nil {
		for _, line := range strings.Split(string(data), "\n") {
			var user, system, idle float64
			if n, _ := fmt.Sscanf(line, "CPU usage: %f%% user, %f%% sys, %f%% idle", &user, &system, &idle); n == 3 {
				out["cpu_user_percent"], out["cpu_system_percent"], out["cpu_idle_percent"] = user, system, idle
				out["cpu_sample_available"] = true
			}
		}
	}
	return out
}

func blobScalingImport(ctx context.Context, level string, rows int) (blobScalingSample, bool) {
	sample := blobScalingSample{Rows: rows}
	w, err := world.New(ctx, level, 0, 0)
	if err != nil {
		return sample, false
	}
	defer w.Close()
	sample.Before = blobHostTelemetry(ctx)
	client := *w.Control
	client.Timeout = 2 * time.Minute
	req, err := http.NewRequestWithContext(ctx, "POST", "http://sim/blob-history", strings.NewReader(fmt.Sprintf(`{"rows":%d}`, rows)))
	if err != nil {
		return sample, false
	}
	response, err := client.Do(req)
	if err != nil {
		return sample, false
	}
	defer response.Body.Close()
	var result struct {
		CommitMS      float64 `json:"commit_ms"`
		RowsPerSecond float64 `json:"rows_per_second"`
	}
	err = json.NewDecoder(response.Body).Decode(&result)
	sample.CaptureMS, sample.RowsPerSecond = result.CommitMS, result.RowsPerSecond
	sample.MSPerCapture = result.CommitMS / float64((rows+63)/64)
	sample.After = blobHostTelemetry(ctx)
	if err != nil || response.StatusCode != 200 || result.RowsPerSecond <= 0 {
		return sample, false
	}
	records, err := w.Records(w.Gateway)
	count := 0
	expected := map[string]bool{}
	for _, r := range records {
		if r.Collection == "reminders.reminder" && strings.HasPrefix(recordTitle(r), "[ab-test] blob-history-") {
			count++
			if refs := contracts.BlobReferences(r.Data); len(refs) == 1 {
				expected[refs[0].Digest] = true
				sample.References++
			}
		}
	}
	if err != nil || count != rows || sample.References != rows/5 {
		return sample, false
	}
	progress, err := blobProgress(ctx, w.Gateway)
	sample.ProtectedReferences = len(progress.Entries)
	if err != nil || sample.ProtectedReferences != rows/5 {
		return sample, false
	}
	for d, e := range progress.Entries {
		if !expected[d] || e.State != "offloaded" {
			return sample, false
		}
	}
	_, err = w.Cursor(w.Gateway)
	return sample, err == nil
}

func blobReferenceScaling(ctx context.Context, w *world.World, evidence *Evidence, add func(string, bool)) bool {
	samples := []blobScalingSample{}
	cost := map[int][]float64{}
	rate := map[int][]float64{}
	for repeat := 0; repeat < 3; repeat++ {
		sizes := []int{2000, 10000, 20000}
		for offset := 0; offset < len(sizes); offset++ {
			rows := sizes[(offset+repeat)%len(sizes)]
			sample, ok := blobScalingImport(ctx, w.Level, rows)
			samples = append(samples, sample)
			evidence.Metrics = map[string]any{"samples": samples, "reference_fraction": 0.2, "capture_rows_per_frame": 64, "capture_replicas": 0, "idle_harness_replicas": len(w.Replicas)}
			add(fmt.Sprintf("reference_heavy_%d_repeat_%d_durable", rows, repeat+1), ok)
			if !ok {
				return false
			}
			cost[rows] = append(cost[rows], sample.MSPerCapture)
			rate[rows] = append(rate[rows], sample.RowsPerSecond)
		}
	}
	medianCost := map[int]float64{}
	medianRate := map[int]float64{}
	for _, rows := range []int{2000, 10000, 20000} {
		slices.Sort(cost[rows])
		slices.Sort(rate[rows])
		medianCost[rows], medianRate[rows] = cost[rows][1], rate[rows][1]
		evidence.Latency[fmt.Sprintf("history_%d_capture", rows)] = stats(cost[rows], 0)
	}
	ratio := medianCost[20000] / medianCost[2000]
	evidence.Metrics["median_ms_per_capture"] = medianCost
	evidence.Metrics["median_rows_per_second"] = medianRate
	evidence.Metrics["capture_cost_20k_over_2k"] = ratio
	evidence.Metrics["capture_cost_ratio_limit"] = 1.25
	passed := ratio <= 1.25
	add("reference_heavy_capture_cost_stays_within_1_25x", passed)
	return passed
}
