package scenarios

import (
	"context"
	"testing"
)

func TestIPhoneRealRequiresSelectedPhysicalDevice(t *testing.T) {
	t.Setenv("AB_REAL_DEVICE", "")
	evidence := runIPhoneReal(context.Background(), Scenario{ID: "APP-CMD-REAL"}, "real")
	if evidence.Passed || len(evidence.Assertions) != 1 || evidence.Assertions[0].Name != "physical_device_selected" {
		t.Fatal("real handler accepted an unselected phone")
	}
}

func TestIPhoneRealRoutesOnlyHardwareScenarios(t *testing.T) {
	for _, id := range []string{"APP-SHELL-REAL", "APP-PAIR-REAL", "APP-CMD-REAL", "APP-NOTIFY-REAL", "APP-APPROVE-REAL", "APP-LIVE-REAL", "APP-INTENTS-REAL"} {
		if !isIPhoneRealScenario(id, "real") || isIPhoneRealScenario(id, "sim") || len(phoneRequirements(id)) == 0 {
			t.Fatalf("incorrect hardware route for %s", id)
		}
	}
	for _, id := range []string{"APP-BUILD-REAL", "APP-CMD-SIM", "APP-HEALTH-REAL"} {
		if isIPhoneRealScenario(id, "real") {
			t.Fatalf("stole unrelated route %s", id)
		}
	}
}

func TestIPhoneRealCannotWriteMacKeychain(t *testing.T) {
	t.Setenv("AB_REAL_DEVICE", "selected")
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "")
	evidence := runIPhoneReal(context.Background(), Scenario{ID: "APP-PAIR-REAL"}, "real")
	if evidence.Passed || evidence.Assertions[0].Name != "isolated_file_secret_store" {
		t.Fatal("phone fixture allowed implicit Mac keychain storage")
	}
}
