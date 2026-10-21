package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runNativeAppCache(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("cache"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Metrics: map[string]any{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if _, err := os.Stat("packaging/dev/signed-app-cache.py"); err != nil {
		add("signed_app_cache_available", false)
		return e
	}
	cache, err := os.MkdirTemp("", "ab-signed-app-cache-")
	if err != nil {
		add("isolated_cache", false)
		return e
	}
	defer os.RemoveAll(cache)
	previous, wasSet := os.LookupEnv("APPLE_BRIDGE_SIGNED_APP_CACHE")
	os.Setenv("APPLE_BRIDGE_SIGNED_APP_CACHE", cache)
	defer func() {
		if wasSet {
			os.Setenv("APPLE_BRIDGE_SIGNED_APP_CACHE", previous)
		} else {
			os.Unsetenv("APPLE_BRIDGE_SIGNED_APP_CACHE")
		}
	}()
	keys := map[string]string{}
	var coldPermissions map[string]string
	for _, round := range []string{"cold", "warm"} {
		started := time.Now()
		load, _ := exec.Command("sysctl", "-n", "vm.loadavg").Output()
		swap, _ := exec.Command("sysctl", "-n", "vm.swapusage").Output()
		e.Metrics[round+"_host_load"] = strings.TrimSpace(string(load))
		e.Metrics[round+"_host_swap"] = strings.TrimSpace(string(swap))
		w, err := world.NewReal(ctx, level, 0)
		if err != nil {
			fmt.Fprintln(os.Stderr, "native cache world:", err)
			add(round+"_world_ready", false)
			return e
		}
		e.Metrics[round+"_world_setup_ms"] = float64(time.Since(started)) / float64(time.Millisecond)
		home, _ := os.UserHomeDir()
		reportPath := filepath.Join(home, "Library", "Application Support", "AppleBridgeDev", e.RunID+"-"+round+"-cache-permissions.json")
		exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "status", "--report", reportPath).Run()
		permissionsData, permissionsErr := os.ReadFile(reportPath)
		os.Remove(reportPath)
		var permissions map[string]string
		add(round+"_helper_status_report", permissionsErr == nil && json.Unmarshal(permissionsData, &permissions) == nil && len(permissions) > 0)
		delete(permissions, "automation_messages")
		delete(permissions, "automation_system_events")
		if round == "cold" {
			coldPermissions = permissions
		} else {
			add("cached_app_preserves_native_tcc_status", reflect.DeepEqual(coldPermissions, permissions))
		}
		for _, role := range []string{"AppleBridgeHelper", "AppleTruth"} {
			data, err := os.ReadFile(filepath.Join(w.Dir, "apps", role+".cache.json"))
			var report struct {
				Hit        bool    `json:"hit"`
				Key        string  `json:"key"`
				BundleHash string  `json:"bundle_sha256"`
				SetupMS    float64 `json:"setup_ms"`
			}
			valid := err == nil && json.Unmarshal(data, &report) == nil && len(report.Key) == 64 && len(report.BundleHash) == 64
			add(round+"_"+role+"_expected_cache_result", valid && report.Hit == (round == "warm"))
			e.Metrics[round+"_"+role+"_setup_ms"] = report.SetupMS
			if round == "cold" {
				keys[role] = report.Key + "/" + report.BundleHash
				e.Metrics[role+"_content_key"] = report.Key
				e.Metrics[role+"_bundle_sha256"] = report.BundleHash
			} else {
				add(role+"_same_content_key", report.Key+"/"+report.BundleHash == keys[role])
			}
			_, err = exec.CommandContext(ctx, "codesign", "--verify", "--strict", filepath.Join(w.Dir, "apps", role+".app")).CombinedOutput()
			add(round+"_"+role+"_signature_valid", err == nil)
		}
		truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": "[ab-test " + e.RunID + "]"})
		add(round+"_native_truth_verified", truthErr == nil && truth["status"] == "verified")
		w.Close()
	}
	add("corrupt_entry_rebuilt_fresh", nativeCacheRecovers(ctx, cache, strings.Split(keys["AppleBridgeHelper"], "/")[0]))
	for n := 0; n < 3; n++ {
		entry := filepath.Join(cache, strings.Repeat(string(rune('a'+n)), 64))
		os.Mkdir(entry, 0700)
		stamp := time.Now().Add(-time.Duration(3-n) * time.Minute)
		os.Chtimes(entry, stamp, stamp)
	}
	_, err = exec.CommandContext(ctx, "python3", "packaging/dev/signed-app-cache.py", "--trim").CombinedOutput()
	entries, _ := os.ReadDir(cache)
	count := 0
	for _, entry := range entries {
		if entry.IsDir() {
			count++
		}
	}
	add("cache_retains_last_three_entries", err == nil && count == 3)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
