package scenarios

import (
	"context"
	"time"
)

func runRemindersStore(_ context.Context, s Scenario, level string) Evidence {
	return Evidence{
		Scenario:   s.ID,
		Level:      level,
		At:         time.Now().UTC(),
		Assertions: []Assertion{{Name: "store_fields_match_tagged_fixture", Passed: false, Detail: "W5-STORE contract skeleton: reader not implemented"}},
		Latency:    map[string]Latency{},
		Hosts:      []string{"air"},
	}
}
