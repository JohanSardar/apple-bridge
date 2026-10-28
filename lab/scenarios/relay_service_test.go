package scenarios

import (
	"context"
	"testing"
)

func TestRelayService(t *testing.T) {
	for _, id := range []string{"RELAY-PASSTHROUGH", "RELAY-LIMITS"} {
		t.Run(id, func(t *testing.T) {
			e := runRelayService(context.Background(), Scenario{ID: id}, "sim")
			for _, a := range e.Assertions {
				if !a.Passed {
					t.Error(a.Name)
				}
			}
		})
	}
}
