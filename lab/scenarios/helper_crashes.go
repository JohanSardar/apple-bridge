package scenarios

import (
	"encoding/json"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

func diagnosticReportsDir() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	return filepath.Join(home, "Library", "Logs", "DiagnosticReports")
}

func helperCrashReports(dir string) (map[string]bool, error) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return nil, err
	}
	reports := map[string]bool{}
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), "AppleBridgeHelper") && strings.HasSuffix(entry.Name(), ".ips") {
			reports[entry.Name()] = true
		}
	}
	return reports, nil
}

func newHelperCrashes(dir string, before map[string]bool) []string {
	after, err := helperCrashReports(dir)
	if err != nil {
		return nil
	}
	added := []string{}
	for name := range after {
		if !before[name] {
			added = append(added, name)
		}
	}
	sort.Strings(added)
	return added
}

func failOnHelperCrashes(e Evidence, dir string, before map[string]bool, pids ...string) Evidence {
	crashes := newHelperCrashes(dir, before)
	if len(pids) > 0 {
		selected := crashes[:0]
		for _, name := range crashes {
			file, err := os.Open(filepath.Join(dir, name))
			var report struct {
				PID int `json:"pid"`
			}
			if err == nil {
				decoder := json.NewDecoder(file)
				var header json.RawMessage
				err = decoder.Decode(&header)
				if err == nil {
					err = decoder.Decode(&report)
				}
				file.Close()
			}
			matching := err != nil || report.PID == 0
			for _, pid := range pids {
				matching = matching || strconv.Itoa(report.PID) == pid
			}
			if matching {
				selected = append(selected, name)
			}
		}
		crashes = selected
	}
	if len(crashes) == 0 {
		return e
	}
	e.Passed = false
	e.Assertions = append(e.Assertions, Assertion{Name: "helper_crash_free", Passed: false, Detail: "new Helper crash reports: " + strings.Join(crashes, ", ")})
	return e
}
