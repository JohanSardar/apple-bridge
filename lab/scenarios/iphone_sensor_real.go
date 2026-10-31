package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"slices"
	"sort"
	"time"
)

func runIPhoneSensorReal(ctx context.Context, s Scenario) Evidence {
	e := Evidence{Scenario: s.ID, Level: "real", At: time.Now().UTC(), Status: "limited"}
	cmd := exec.CommandContext(ctx, "python3", "lab/iphone-real/run.py", s.ID)
	cmd.Stderr = os.Stderr
	raw, err := cmd.Output()
	var result struct {
		Checks  map[string]bool   `json:"checks"`
		Counts  map[string]int    `json:"counts"`
		Digests map[string]string `json:"digests"`
	}
	if json.Unmarshal(raw, &result) != nil || len(result.Checks) == 0 {
		e.Assertions = []Assertion{{Name: "real_device_result", Detail: "No valid real iPhone result"}}
		return e
	}
	e.Passed = err == nil
	names := make([]string, 0, len(result.Checks))
	for name := range result.Checks {
		names = append(names, name)
	}
	sort.Strings(names)
	for _, name := range names {
		passed := result.Checks[name]
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	summary, _ := json.Marshal(struct {
		Counts  map[string]int    `json:"counts"`
		Digests map[string]string `json:"digests"`
	}{result.Counts, result.Digests})
	e.Assertions = append(e.Assertions, Assertion{Name: "redacted_observation", Passed: true, Detail: string(summary)})
	if e.Passed {
		e.Status = "passed"
	}
	return e
}

func isIPhoneSensorReal(id string) bool {
	return slices.Contains([]string{"APP-HEALTH-REAL", "APP-HEALTH-WRITE-REAL", "APP-MOTION-REAL", "APP-ALARM-REAL", "APP-DEVICE-REAL", "APP-SCREENTIME-REAL", "APP-LOC-REAL", "APP-CAPTURE-REAL"}, id)
}
