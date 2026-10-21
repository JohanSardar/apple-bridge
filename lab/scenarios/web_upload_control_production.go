package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os/exec"
	"strings"
	"time"
)

func runUploadControlProduction(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("control"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Passed: true, Hosts: []string{"air"}}
	if uploadControlLabEnabled() {
		return runUploadTagged(ctx, s, level, false)
	}
	control, secret := webBrowserControl(ctx, nil, "")
	defer control.Close()
	for _, path := range []string{"/replica-upload-checkpoint/arm", "/replica-upload-checkpoint/wait", "/replica-upload-checkpoint/release", "/command-effect"} {
		status, _ := uploadCheckpointRequest(ctx, control.URL, secret, path, map[string]string{"upload_id": "upload_production"})
		ok := status == 404
		e.Assertions = append(e.Assertions, Assertion{Name: "non_lab_route_absent:" + path, Passed: ok})
		e.Passed = e.Passed && ok
	}
	return e
}

func runUploadTagged(ctx context.Context, s Scenario, level string, lab bool) Evidence {
	args := []string{"run"}
	if lab {
		args = append(args, "-tags", "lab")
	}
	args = append(args, "./cmd/bridge-lab", "run", "lab/scenarios/web/"+s.ID+".yaml", "--level", level, "--json")
	output, _ := exec.CommandContext(ctx, "go", args...).Output()
	for _, line := range strings.Split(string(output), "\n") {
		var result Evidence
		if json.Unmarshal([]byte(line), &result) == nil && result.Scenario == s.ID && result.RunID != "" {
			return result
		}
	}
	return Evidence{RunID: contracts.NewID("control"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Assertions: []Assertion{{Name: "tagged_runner_result", Passed: false}}, Hosts: []string{"air"}}
}
