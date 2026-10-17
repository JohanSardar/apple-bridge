package agent

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"math"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func numberValue(value any) (float64, bool) {
	switch value := value.(type) {
	case float64:
		return value, true
	case json.Number:
		parsed, err := value.Float64()
		return parsed, err == nil
	default:
		return 0, false
	}
}

func (r AgentRunner) runUsage(ctx context.Context, task Task, number int) (attempt Attempt) {
	start := time.Now()
	attempt.Number = number
	attempt.Cleanup = true
	defer func() { attempt.Duration = time.Since(start).Milliseconds() }()
	ctx, cancel := context.WithTimeout(ctx, 8*time.Minute)
	defer cancel()
	w, err := world.NewRealWithSetup(ctx, "replica", 1, world.NativeSetup{BeforeStart: func(w *world.World) error { return w.StageUsageTruth() }}, world.BlobOptions{Scope: "usage:read,system:read"})
	if err != nil {
		attempt.Failure = "usage_world"
		return
	}
	defer w.Close()
	truth, err := w.Truth("observe", map[string]any{"source": "usage", "biome_root": w.UsageTruthRoot()})
	if err != nil {
		attempt.Failure = "independent_usage_truth"
		return
	}
	rows, _ := truth["rows"].(map[string]any)
	var chosen contracts.Record
	var fields map[string]any
	var expected float64
	endpoint := w.Replicas[0]
	if w.Await(45*time.Second, func() bool {
		after := ""
		for {
			var page contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "usage.app_day", IncludeAliases: true, Limit: 1000, After: after}, &page)
			if err != nil {
				return false
			}
			keys := map[string]string{}
			for _, alias := range page.Aliases {
				if alias.Collection == "usage.app_day" {
					sum := sha256.Sum256([]byte(alias.NativeKey))
					keys[alias.RecordID] = hex.EncodeToString(sum[:])
				}
			}
			for _, record := range page.Records {
				var data map[string]any
				if json.Unmarshal(record.Data, &data) != nil {
					continue
				}
				day, _ := data["day"].(string)
				if day == "" || day >= time.Now().Format("2006-01-02") {
					continue
				}
				row, _ := rows[keys[record.ID]].(map[string]any)
				want, ok := numberValue(row["seconds_estimated"])
				got, gotOK := numberValue(data["seconds_estimated"])
				if ok && gotOK && want > 0 && math.Abs(want-got) < 0.001 {
					var replica contracts.ReadResult
					_, readErr := world.Request(ctx, endpoint, "/v1/op/records.get", contracts.ReadQuery{Collection: "usage.app_day", ID: record.ID}, &replica)
					if readErr != nil || len(replica.Records) != 1 {
						continue
					}
					var replicaFields map[string]any
					if json.Unmarshal(replica.Records[0].Data, &replicaFields) != nil {
						continue
					}
					value, present := numberValue(replicaFields["seconds_estimated"])
					if !present || math.Abs(value-want) >= 0.001 {
						continue
					}
					chosen = record
					fields = data
					expected = want
					return true
				}
			}
			if page.Next == "" {
				return false
			}
			after = page.Next
		}
	}) != nil {
		attempt.Failure = "stable_usage_entry_matches_truth"
		return
	}
	prompt := strings.NewReplacer("{{record}}", chosen.ID, "{{bundle}}", stringField(fields, "bundle_id"), "{{day}}", stringField(fields, "day"), "{{device}}", stringField(fields, "device_id")).Replace(task.Prompt)
	prompt = "Use only apple_bridge MCP tools. Never use shell, file, browser, or other tools. This is a read-only task.\n" + prompt
	result, err := r.execute(ctx, w, task, number, prompt, []string{"records_get", "records_query", "sources_status"})
	if err != nil {
		attempt.Failure = "codex_execution"
		return
	}
	attempt.MCPOnly = result.Exit == 0 && result.Calls > 0 && result.Forbidden == 0
	answer, parseErr := strconv.ParseFloat(strings.TrimSpace(result.Answer), 64)
	attempt.Truth = parseErr == nil && !math.IsNaN(answer) && !math.IsInf(answer, 0) && math.Abs(answer-expected) < 0.001
	var found contracts.ReadResult
	_, err = world.Request(ctx, endpoint, "/v1/op/records.get", contracts.ReadQuery{Collection: "usage.app_day", ID: chosen.ID}, &found)
	if err == nil && len(found.Records) == 1 {
		var current map[string]any
		if json.Unmarshal(found.Records[0].Data, &current) == nil {
			value, ok := numberValue(current["seconds_estimated"])
			attempt.Replica = ok && math.Abs(value-expected) < 0.001 && current["day"] == fields["day"] && current["device_id"] == fields["device_id"] && current["bundle_id"] == fields["bundle_id"]
		}
	}
	attempt.Passed = attempt.Truth && attempt.Replica && attempt.MCPOnly
	return
}

func stringField(fields map[string]any, key string) string {
	value, _ := fields[key].(string)
	return value
}
