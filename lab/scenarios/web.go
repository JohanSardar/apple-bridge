package scenarios

import (
	"context"
	"fmt"
	"slices"
	"time"
)

type WebOptions struct {
	Fixture  string   `yaml:"fixture,omitempty"`
	Huge     bool     `yaml:"huge,omitempty"`
	Browsers []string `yaml:"browsers"`
	Themes   []string `yaml:"themes"`
	Layouts  []string `yaml:"layouts"`
	Server   string   `yaml:"server"`
}

func validateWeb(s Scenario) error {
	if !slices.Contains(s.Levels, "web") || s.Web == nil || len(s.Web.Browsers)*len(s.Web.Themes)*len(s.Web.Layouts) == 0 || (s.Web.Server != "mock" && s.Web.Server != "gateway") {
		return fmt.Errorf("web scenario requires matrix and server")
	}
	if s.Web.Fixture != "" && (s.Web.Fixture != "list" || s.Web.Server != "gateway") {
		return fmt.Errorf("invalid web fixture %q", s.Web.Fixture)
	}
	for _, pair := range []struct{ values, allowed []string }{{s.Web.Browsers, []string{"webkit", "chromium", "firefox"}}, {s.Web.Themes, []string{"light", "dark"}}, {s.Web.Layouts, []string{"phone", "desktop"}}} {
		for _, value := range pair.values {
			if !slices.Contains(pair.allowed, value) {
				return fmt.Errorf("invalid web matrix value %q", value)
			}
		}
	}
	return nil
}

const webProjectBudget = 3 * time.Minute
const webFixtureBuildBudget = 6 * time.Minute

func webSnapshotEnabled(s Scenario) bool {
	return s.Web != nil && s.Web.Server == "gateway" && (s.Web.Huge || s.Web.Fixture == "list")
}
func webRunBudget(s Scenario) time.Duration {
	if webSnapshotEnabled(s) {
		return webProjectBudget + webFixtureBuildBudget
	}
	return webProjectBudget
}

func runWeb(ctx context.Context, s Scenario) Evidence {
	return Evidence{Scenario: s.ID, Level: "web", Assertions: []Assertion{{Name: "SolidJS scenarios live on web-app", Passed: false}}}
}
