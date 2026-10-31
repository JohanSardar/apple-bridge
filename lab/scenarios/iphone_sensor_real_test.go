package scenarios

import (
	"context"
	"testing"
)

func TestIPhoneSensorRealRequiresDeviceEvidence(t *testing.T) {
	t.Chdir("../..")
	t.Setenv("APPLE_BRIDGE_REAL_DEVICE", "")
	for _, id := range []string{"APP-HEALTH-REAL", "APP-HEALTH-WRITE-REAL", "APP-MOTION-REAL", "APP-ALARM-REAL", "APP-DEVICE-REAL", "APP-SCREENTIME-REAL", "APP-LOC-REAL", "APP-CAPTURE-REAL"} {
		t.Run(id, func(t *testing.T) {
			e := runScenario(context.Background(), Scenario{ID: id}, "real")
			if e.Passed || e.Status != "limited" || len(e.Assertions) != 1 || e.Assertions[0].Name != "real_device_result" {
				t.Fatalf("missing device evidence must fail closed: %+v", e)
			}
		})
	}
}
