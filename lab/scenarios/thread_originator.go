package scenarios

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runThreadOriginator(ctx context.Context, scenario Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	budgets := strings.HasSuffix(scenario.ID, "-PERF")
	baseMessages := 750
	datasetDescription := "canonical demo plus bounded synthetic gaps and distant replies"
	if budgets {
		baseMessages = 100000
		datasetDescription = "canonical huge demo plus synthetic distant thread replies"
	}
	e := Evidence{RunID: contracts.NewID("run"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}, Conditions: map[string]string{"dataset": datasetDescription, "transport": "REST, MCP and authenticated web.1"}, Metrics: map[string]any{"archive_base_messages": baseMessages, "archive_messages": baseMessages + 206, "filtered_items": 206, "page_limit": 200}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "messages:read,contacts:read,system:read"})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	fixtureStart := time.Now()
	fixture, err := populateThreadFixture(ctx, w, budgets)
	e.Metrics["fixture_setup_ms"] = float64(time.Since(fixtureStart)) / float64(time.Millisecond)
	add("canonical_demo_and_distant_thread_captured", err == nil)
	if err != nil {
		fmt.Fprintf(os.Stderr, "originator fixture: %v\n", err)
		return e
	}
	e.Metrics["budget_status"] = "deferred_to_messages_milestone"
	if budgets {
		e.Metrics["budget_status"] = "measured"
	}
	checkThreadOriginator(ctx, w, fixture, &e, add, budgets)
	checkThreadOriginatorLive(ctx, w, fixture, add)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
