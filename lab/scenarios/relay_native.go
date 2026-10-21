package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

const relayLiveSamples = 200

func relayNativeChecks(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	tag := "[ab-test " + contracts.NewID("relay") + "]"
	_, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": tag})
	add("native_reminders_truth_ready", err == nil)
	if err != nil {
		e.Status = "blocked"
		e.Assertions[len(e.Assertions)-1].Detail = "AppleTruth Reminders access is unverified; no fixture changes made"
		return
	}
	defer func() {
		_, err := w.RecoverTruth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": tag})
		add("tagged_native_fixture_cleanup", err == nil)
	}()
	_, err = w.Truth("fixture", map[string]any{"action": "create_list", "list": "Bridge Test", "tag": tag})
	add("bridge_test_list_ready", err == nil)
	if err != nil {
		return
	}
	relayNativeFunctionalChecks(w, e, add, tag)
	gatewayEndpoint := w.Gateway
	gatewayEndpoint.Token, err = w.ScopedToken(w.Gateway, "reminders:read,system:read")
	add("gateway_observer_scope", err == nil)
	if err != nil {
		return
	}
	gatewayObserver, err := watch(ctx, gatewayEndpoint, false, func(contracts.Record) bool { return true })
	add("gateway_live_observer", err == nil)
	if err != nil {
		return
	}
	defer gatewayObserver.close()
	var effects []struct {
		Title    string
		WallNS   int64
		UptimeNS int64
	}
	liveInput := relayProbeInput(w)
	liveInput["Prefix"] = tag
	var live struct {
		Values      []float64 `json:"latencies_ms"`
		Uncertainty float64   `json:"clock_uncertainty_ms"`
	}
	err = w.RelayExchange("live", liveInput, func() (any, error) {
		burst, err := w.Truth("fixture", map[string]any{"action": "burst", "list": "Bridge Test", "tag": tag, "count": relayLiveSamples})
		if err != nil {
			return nil, err
		}
		raw, ok := burst["effects"].([]any)
		if !ok || len(raw) != relayLiveSamples {
			return nil, fmt.Errorf("native effects missing")
		}
		out := []map[string]any{}
		for _, value := range raw {
			effect, ok := value.(map[string]any)
			if !ok {
				return nil, fmt.Errorf("native effect malformed")
			}
			title, _ := effect["title"].(string)
			number, ok := effect["effect_wall_ns"].(json.Number)
			if !ok || !strings.HasPrefix(title, tag) {
				return nil, fmt.Errorf("native effect timing malformed")
			}
			wall, err := number.Float64()
			if err != nil || wall <= 0 {
				return nil, fmt.Errorf("native wall clock invalid")
			}
			uptimeNumber, ok := effect["effect_uptime_ns"].(json.Number)
			if !ok {
				return nil, fmt.Errorf("native monotonic clock missing")
			}
			uptime, err := uptimeNumber.Int64()
			if err != nil || uptime <= 0 {
				return nil, fmt.Errorf("native monotonic clock invalid")
			}
			out = append(out, map[string]any{"title": title, "wall_ns": int64(wall)})
			effects = append(effects, struct {
				Title    string
				WallNS   int64
				UptimeNS int64
			}{title, int64(wall), uptime})
		}
		return out, nil
	}, &live)
	add("native_effects_reach_ubuntu_local_feed", err == nil && len(live.Values) == relayLiveSamples)
	if err != nil {
		return
	}
	gatewayLatencies, relayLatencies := []float64{}, []float64{}
	for index, effect := range effects {
		seen, ok := gatewayObserver.observedExact(effect.Title)
		if !ok {
			add("paired_gateway_observations", false)
			return
		}
		elapsed := float64(seen.UptimeNS-effect.UptimeNS) / 1e6
		gatewayLatencies = append(gatewayLatencies, elapsed)
		relayLatencies = append(relayLatencies, max(0, live.Values[index]-elapsed))
	}
	add("paired_gateway_observations", len(gatewayLatencies) == relayLiveSamples)
	e.Latency["apple_effect→gateway_local_feed"] = stats(gatewayLatencies, 0)
	e.Latency["gateway_local_feed→ubuntu_local_feed"] = stats(relayLatencies, 0)
	latency := stats(live.Values, 0)
	e.Latency["apple_effect→ubuntu_local_feed"] = latency
	e.Metrics["relay_clock_uncertainty_ms"] = live.Uncertainty
	e.Metrics["live_samples"] = relayLiveSamples
	e.Metrics["relay_R_LIVE_budget_ms"] = 1000
	e.Metrics["relay_R_LIVE_budget_met"] = latency.P95+live.Uncertainty < 1000
	add("relay_R_LIVE_gateway_under_one_second", e.Latency["apple_effect→gateway_local_feed"].P95 < 1000 && len(gatewayLatencies) == relayLiveSamples)
	add("relay_R_LIVE_certification", latency.P95+live.Uncertainty < 1000 && len(live.Values) == relayLiveSamples)
}
