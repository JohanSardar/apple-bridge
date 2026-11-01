package scenarios

import (
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"testing"
)

func TestRealPhoneConnectorRequiresMatchingSingleUseDecision(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	authority, err := trust.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	fixture, err := newRealPhoneConnector(authority)
	if err != nil {
		t.Fatal(err)
	}
	if fixture.qualified() || len(fixture.pendingPhone()) != 1 {
		t.Fatal("pending request qualified without a phone decision")
	}
	if fixture.approvals.Decide(fixture.pending.ID, "wrong", true) == nil || fixture.qualified() {
		t.Fatal("wrong matching code authorized a connector")
	}
	if err := fixture.approvals.Decide(fixture.pending.ID, fixture.pending.Code, true); err != nil {
		t.Fatal(err)
	}
	if !fixture.qualified() || fixture.qualified() || len(fixture.pendingPhone()) != 0 {
		t.Fatal("connector token was missing or approval was reused")
	}
}
