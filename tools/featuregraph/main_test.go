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
