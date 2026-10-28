package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	bridgeruntime "github.com/JohanSardar/apple-bridge/internal/runtime"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func runPermissionStatus(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC()}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	app := os.Getenv("AB_PERMISSION_HELPER_APP")
	fingerprint := func() (string, bool) {
		var values []string
		for _, file := range []string{filepath.Join(os.Getenv("HOME"), "Library/Application Support/com.apple.TCC/TCC.db"), "/Library/Application Support/com.apple.TCC/TCC.db"} {
			for _, suffix := range []string{"", "-wal"} {
				data, err := os.ReadFile(file + suffix)
				if os.IsNotExist(err) && suffix != "" {
					continue
				}
				if err != nil {
					return "", false
				}
				values = append(values, fmt.Sprintf("%x", sha256.Sum256(data)))
			}
		}
		return strings.Join(values, ":"), true
	}
	before, readable := fingerprint()
	var states map[string]string
	var err error
	if level == "sim" {
		states, err = bridgeruntime.Permissions(ctx, app, "fake")
	} else {
		states, err = bridgeruntime.PermissionStatusProof(ctx, app)
	}
	metrics := map[string]any{}
	if level == "real" {
		calls, countErr := strconv.Atoi(states["_request_api_calls"])
		metrics["request_api_calls"] = calls
		add("zero_native_request_api_calls", countErr == nil && calls == 0)
		counts := map[string]map[string]int{}
		for _, phase := range []string{"before", "after", "peak"} {
			var values map[string]int
			decodeErr := json.Unmarshal([]byte(states["_windows_"+phase]), &values)
			add("window_observation_"+phase, decodeErr == nil && values != nil)
			counts[phase] = values
			delete(states, "_windows_"+phase)
		}
		noCreation := true
		for owner, count := range counts["peak"] {
			noCreation = noCreation && count <= counts["before"][owner]
		}
		add("no_new_dialog_windows", noCreation)
		metrics["window_owner_counts"] = counts
		delete(states, "_request_api_calls")
	}
	add("helper_report", err == nil && (level == "real" && len(states) == 13 || level == "sim" && len(states) == 14))
	for key, state := range states {
		valid := state == "not_determined" || state == "denied" || state == "authorized" || state == "unknown"
		if level == "sim" {
			valid = state == "1"
		}
		add(key, valid)
	}
	if level == "real" {
		after, ok := fingerprint()
		add("tcc_database_readable", readable && ok)
		add("tcc_database_unchanged", readable && before == after)
		metrics["states"] = states
	}
	e.Metrics = metrics
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return
}
