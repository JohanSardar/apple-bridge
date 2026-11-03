package scenarios

import (
	"context"
	"testing"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func TestRelayService(t *testing.T) {
	for _, id := range []string{"RELAY-ENROLL", "RELAY-PASSTHROUGH", "RELAY-LIMITS"} {
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

func TestRelayServiceRustSelection(t *testing.T) {
	ctx := world.WithImplementations(context.Background(), world.Implementations{Gateway: "rust", Replica: "go"})
	for _, id := range []string{"RELAY-ENROLL", "RELAY-PASSTHROUGH", "RELAY-LIMITS"} {
		t.Run(id, func(t *testing.T) {
			e := Run(ctx, Scenario{ID: id, Feature: "remote.relay"}, "sim")
			if !e.Passed || len(e.Assertions) < 10 {
				t.Fatalf("relay assertions did not pass: %+v", e)
			}
		})
	}
}
