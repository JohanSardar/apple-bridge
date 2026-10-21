package scenarios

import (
	"context"
	"time"
)

func runRemindersStorePerf(_ context.Context, s Scenario, level string) Evidence {
	return Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Assertions: []Assertion{{Name: "profile_then_warm_incremental_read_under_50_ms", Passed: false, Detail: "cold and 200 warm iterations not profiled"}}, Latency: map[string]Latency{}}
}
