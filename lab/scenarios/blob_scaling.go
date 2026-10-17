package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type blobScalingSample struct {
	Rows                int     `json:"rows"`
	ProtectedReferences int     `json:"protected_references"`
	References          int     `json:"references"`
	CaptureMS           float64 `json:"capture_ms"`
	RowsPerSecond       float64 `json:"rows_per_second"`
	MSPerCapture        float64 `json:"ms_per_capture"`
}

func blobScalingImport(ctx context.Context, level string, rows int) (blobScalingSample, bool) {
	sample := blobScalingSample{Rows: rows}
	w, err := world.New(ctx, level, 0, 0)
	if err != nil {
		return sample, false
	}
	defer w.Close()
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
	host := messagesHostLoad()
	samples := []blobScalingSample{}
	costs := map[int][]float64{}
	for i, rows := range []int{2000, 20000, 2000} {
		sample, ok := blobScalingImport(ctx, w.Level, rows)
		samples = append(samples, sample)
		evidence.Metrics = map[string]any{"samples": samples, "host_load": host, "reference_fraction": 0.2, "capture_rows_per_frame": 64, "capture_replicas": 0, "idle_harness_replicas": len(w.Replicas)}
		add(fmt.Sprintf("reference_heavy_%d_sample_%d_durable", rows, i+1), ok)
		if !ok {
			return false
		}
		costs[rows] = append(costs[rows], sample.MSPerCapture)
	}
	baseline := (costs[2000][0] + costs[2000][1]) / 2
	ratio := costs[20000][0] / baseline
	evidence.Metrics["small_bracketing_mean_ms_per_capture"] = baseline
	evidence.Metrics["capture_cost_20k_over_2k"] = ratio
	evidence.Metrics["capture_cost_ratio_limit"] = 1.25
	for rows, samples := range costs {
		evidence.Latency[fmt.Sprintf("history_%d_capture", rows)] = stats(samples, 0)
	}
	passed := ratio <= 1.25
	add("reference_heavy_capture_cost_stays_within_1_25x", passed)
	return passed
}
