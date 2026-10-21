package scenarios

import (
	"context"
	"encoding/json"
	"math"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelPerf(ctx context.Context, e *Evidence, add func(string, bool)) {
	w, err := world.New(ctx, "sim", 0, 1000, world.BlobOptions{Scope: "reminders:read,system:read"})
	add("1000_reminders_perf_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	add("1000_reminders_perf_convergence", w.Await(20*time.Second, func() bool { return convergence(w) }) == nil)
	records, err := w.Records(w.Gateway)
	listID := ""
	for _, record := range records {
		if record.Collection == "reminders.list" {
			listID = record.ID
		}
	}
	add("1000_reminders_perf_list", err == nil && listID != "")
	query := map[string]any{"list": listID, "time_zone": "America/New_York"}
	for _, protocol := range []string{"rest", "mcp"} {
		endpoint := w.Gateway
		endpoint.Token, err = w.ScopedToken(w.Gateway, "reminders:read,system:read")
		add(protocol+"_perf_token", err == nil)
		if err != nil {
			continue
		}
		var headers http.Header
		var body []byte
		client := scripted.Client{Endpoint: endpoint, Protocol: protocol, Observe: func(h http.Header, b []byte) { headers, body = h, b }}
		var page struct{ Count int }
		err = client.Call(ctx, "reminders.items", query, &page)
		add(protocol+"_perf_exactly_1000_items", err == nil && page.Count == 1000)
		if err != nil || page.Count != 1000 {
			continue
		}
		values, serverValues := []float64{}, []float64{}
		failures, missingTimings, longestText, responseBytes, invalidText := 0, 0, 0, 0, 0
		for index := 0; index < 100; index++ {
			started := time.Now()
			err = client.Call(ctx, "reminders.items", query, nil)
			milliseconds := float64(time.Since(started)) / float64(time.Millisecond)
			if err != nil {
				failures++
			} else {
				values = append(values, milliseconds)
				server, textLength, known := reminderHandlerTiming(protocol, headers, body)
				if known {
					serverValues = append(serverValues, server)
				} else {
					missingTimings++
				}
				if protocol == "mcp" && !reminderMCPTextEqual(body) {
					invalidText++
				}
				longestText = max(longestText, textLength)
				responseBytes = max(responseBytes, len(body))
			}
		}
		latency := stats(values, failures)
		e.Latency["1000_reminders_items_"+protocol] = latency
		server := stats(serverValues, missingTimings)
		e.Latency["1000_reminders_items_"+protocol+"_gateway"] = server
		add(protocol+"_1000_items_gateway_p95_under_10ms", server.Samples == 100 && server.Timeouts == 0 && server.P95 < 10)
		add(protocol+"_1000_items_roundtrip_complete", latency.Samples == 100 && latency.Timeouts == 0)
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		e.Metrics[protocol+"_1000_items_response_bytes"] = responseBytes
		if protocol == "mcp" {
			add("mcp_large_result_text_matches_structured_json", longestText > 0 && invalidText == 0)
			e.Metrics["mcp_text_bytes"] = longestText
		}
	}
}

func reminderHandlerTiming(protocol string, headers http.Header, body []byte) (float64, int, bool) {
	var value float64
	textLength := 0
	if protocol == "mcp" {
		var response struct {
			Result struct {
				Meta struct {
					HandlerMS *float64 `json:"bridge/read_model_ms"`
				} `json:"_meta"`
				Content []struct {
					Text string `json:"text"`
				} `json:"content"`
			} `json:"result"`
		}
		if json.Unmarshal(body, &response) != nil {
			return 0, 0, false
		}
		if response.Result.Meta.HandlerMS == nil {
			return 0, 0, false
		}
		value = *response.Result.Meta.HandlerMS
		for _, content := range response.Result.Content {
			textLength += len(content.Text)
		}
	} else {
		var err error
		value, err = strconv.ParseFloat(strings.TrimPrefix(headers.Get("Server-Timing"), "readmodel;dur="), 64)
		if err != nil {
			return 0, 0, false
		}
	}
	return value, textLength, value > 0 && !math.IsNaN(value) && !math.IsInf(value, 0)
}
