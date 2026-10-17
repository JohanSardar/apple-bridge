package scenarios

import (
	"math"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func clockDelay(w *world.World, e *Evidence, add func(string, bool)) bool {
	var baseline map[string]any
	ready := w.Await(5*time.Second, func() bool {
		var err error
		baseline, err = w.Doctor(w.Replicas[0])
		return err == nil && baseline["clock_method"] == "authenticated_feed_ntp_four_timestamps"
	}) == nil
	add("authenticated_four_timestamp_clock_sample", ready)
	if !ready {
		return false
	}
	baseOffset, offsetOK := baseline["clock_offset_ms"].(float64)
	baseUncertainty, uncertaintyOK := baseline["clock_uncertainty_ms"].(float64)
	baseRTT, rttOK := baseline["clock_rtt_ms"].(float64)
	if !offsetOK || !uncertaintyOK || !rttOK {
		add("clock_estimate_fields", false)
		return false
	}
	w.DelayBothDirections(2 * time.Second)
	defer w.DelayBothDirections(0)
	var delayed map[string]any
	ready = w.Await(25*time.Second, func() bool {
		var err error
		delayed, err = w.Doctor(w.Replicas[0])
		rtt, ok := delayed["clock_rtt_ms"].(float64)
		return err == nil && ok && rtt >= 3500
	}) == nil
	add("authenticated_probe_crossed_delayed_feed", ready)
	if !ready {
		return false
	}
	offset, offsetOK := delayed["clock_offset_ms"].(float64)
	uncertainty, uncertaintyOK := delayed["clock_uncertainty_ms"].(float64)
	rtt, rttOK := delayed["clock_rtt_ms"].(float64)
	add("offset_stays_near_zero_as_transit_grows", offsetOK && math.Abs(baseOffset) < 250 && math.Abs(offset) < 250 && math.Abs(offset-baseOffset) < 250)
	add("uncertainty_tracks_round_trip_delay", uncertaintyOK && rttOK && uncertainty > baseUncertainty+1500 && math.Abs(2*uncertainty-rtt) <= 2 && rtt > baseRTT+3000)
	add("transit_not_misreported_as_clock_skew", delayed["clock_skew"] != "warning_clock_skew")
	e.Metrics = map[string]any{"injected_delay_each_direction_ms": 2000, "baseline_offset_ms": baseOffset, "baseline_uncertainty_ms": baseUncertainty, "baseline_rtt_ms": baseRTT, "delayed_offset_ms": offset, "delayed_uncertainty_ms": uncertainty, "delayed_rtt_ms": rtt, "clock_method": delayed["clock_method"]}
	return true
}
