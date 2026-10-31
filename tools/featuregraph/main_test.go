package main

import (
	"os"
	"path/filepath"
	"slices"
	"testing"

	"gopkg.in/yaml.v3"
)

func TestWebScenarioLevelsMatchGraph(t *testing.T) {
	c, err := load("../..")
	if err != nil {
		t.Fatal(err)
	}
	levels := map[string][]string{}
	for _, feature := range c.graph.Features {
		for _, scenario := range feature.Scenarios {
			levels[scenario.ID] = slices.Clone(scenario.Levels)
		}
	}
	paths, err := filepath.Glob("../../lab/scenarios/web/*.yaml")
	if err != nil {
		t.Fatal(err)
	}
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			t.Fatal(err)
		}
		var scenario scenarioRef
		if err := yaml.Unmarshal(raw, &scenario); err != nil {
			t.Fatal(err)
		}
		want := levels[scenario.ID]
		slices.Sort(want)
		slices.Sort(scenario.Levels)
		if !slices.Equal(scenario.Levels, want) {
			t.Errorf("%s: manifest levels %v differ from graph levels %v", scenario.ID, scenario.Levels, want)
		}
	}
}

func TestDroppedQualification(t *testing.T) {
	for _, tc := range []struct {
		name, featureStatus, featureReason, scenarioStatus, scenarioReason string
		scenario, wantProblem                                              bool
	}{
		{name: "dropped_feature", featureStatus: "dropped", featureReason: "Outside scope"},
		{name: "feature_requires_reason", featureStatus: "dropped", wantProblem: true},
		{name: "active_feature_requires_scenarios", featureStatus: "done", wantProblem: true},
		{name: "dropped_scenario", featureStatus: "done", scenario: true, scenarioStatus: "dropped", scenarioReason: "Restricted entitlement"},
		{name: "scenario_requires_reason", featureStatus: "done", scenario: true, scenarioStatus: "dropped", wantProblem: true},
		{name: "active_scenario_requires_proof", featureStatus: "done", scenario: true, wantProblem: true},
		{name: "unknown_scenario_status", featureStatus: "done", scenario: true, scenarioStatus: "ignored", wantProblem: true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			root := t.TempDir()
			f := feature{ID: "example", Status: tc.featureStatus, Notes: tc.featureReason, Requirements: []string{"R-API"}}
			if tc.scenario {
				dir := filepath.Join(root, "lab/scenarios")
				if err := os.MkdirAll(dir, 0700); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(filepath.Join(dir, "EXAMPLE.yaml"), []byte("id: EXAMPLE\nfeature: example\nlevels: [sim]\n"), 0600); err != nil {
					t.Fatal(err)
				}
				f.Scenarios = []scenarioRef{{ID: "EXAMPLE", Levels: []string{"sim"}, Status: tc.scenarioStatus, Reason: tc.scenarioReason}}
			}
			c := checker{root: root, graph: graph{Statuses: []string{"done", "dropped"}, Levels: map[string]string{"sim": "simulation"}, Requirements: map[string]string{"R-API": "API"}, Features: []feature{f}}}
			views := c.check()
			if (len(c.problems) > 0) != tc.wantProblem {
				t.Fatalf("problems=%v wantProblem=%t", c.problems, tc.wantProblem)
			}
			if !tc.wantProblem && tc.scenario && views[0].Views[0].Results[0].State != "dropped" {
				t.Fatal("dropped qualification must not count as passing evidence")
			}
		})
	}
}
