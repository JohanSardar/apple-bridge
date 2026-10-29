package contracts

import "time"

func init() {
	RegisterSource("health", "Health", true)
	Collections["health.sample"] = "hsm"
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{{Name: "health.summary", Version: 1, Kind: ReadOp, Source: "health", Scope: "health:read", Input: HealthSummaryQuery{}, Output: HealthSummary{}, Description: "Day or week summary of captured Health samples, grouped by type and unit; durations clipped to the window, quantities assigned by start time"}}
	})
}

type HealthSample struct {
	ID       string         `json:"id"`
	Type     string         `json:"type"`
	Start    time.Time      `json:"start"`
	End      time.Time      `json:"end"`
	Value    *float64       `json:"value,omitempty"`
	Unit     string         `json:"unit,omitempty"`
	Category *int           `json:"category,omitempty"`
	Details  map[string]any `json:"details,omitempty"`
}

type HealthBatch struct {
	Stream   string         `json:"stream"`
	Sequence uint64         `json:"sequence"`
	Samples  []HealthSample `json:"samples"`
	Deleted  []string       `json:"deleted"`
}

type HealthSummaryQuery struct {
	Period   string `json:"period"`
	Date     string `json:"date"`
	TimeZone string `json:"time_zone,omitempty"`
	DeviceID string `json:"device_id,omitempty"`
}

type HealthMetric struct {
	Count   int     `json:"count"`
	Sum     float64 `json:"sum"`
	Average float64 `json:"average"`
	Min     float64 `json:"min"`
	Max     float64 `json:"max"`
	Seconds float64 `json:"seconds"`
}

type HealthSummary struct {
	Start   time.Time               `json:"start"`
	End     time.Time               `json:"end"`
	Metrics map[string]HealthMetric `json:"metrics"`
	Cursor  Cursor                  `json:"cursor"`
}
