package main

import (
	"encoding/json"
	"io/fs"
	"os"
	"path/filepath"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/scenarios"
	"gopkg.in/yaml.v3"
)

func inventory(paths []string) error {
	if len(paths) == 0 {
		paths = []string{"lab/scenarios"}
	}
	entries := []scenarios.Scenario{}
	for _, root := range paths {
		err := filepath.WalkDir(root, func(path string, entry fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if entry.IsDir() || !strings.HasSuffix(path, ".yaml") {
				return nil
			}
			raw, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			var scenario scenarios.Scenario
			if err = yaml.Unmarshal(raw, &scenario); err != nil {
				return err
			}
			if len(scenario.Levels) == 0 && scenario.Level != "" {
				scenario.Levels = []string{scenario.Level}
			}
			entries = append(entries, scenario)
			return nil
		})
		if err != nil {
			return err
		}
	}
	raw, err := os.ReadFile("features.yaml")
	if err != nil {
		return err
	}
	var features any
	if err = yaml.Unmarshal(raw, &features); err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"scenarios": entries, "features": features})
}
