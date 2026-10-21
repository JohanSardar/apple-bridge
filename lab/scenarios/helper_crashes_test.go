package scenarios

import (
	"os"
	"path/filepath"
	"testing"
)

func TestHelperCrashDuringRunFailsEvidence(t *testing.T) {
	dir := t.TempDir()
	write := func(name string) {
		if err := os.WriteFile(filepath.Join(dir, name), []byte("{}"), 0600); err != nil {
			t.Fatal(err)
		}
	}
	write("AppleBridgeHelper-2026-10-21-090423.ips")
	write("Safari-2026-10-21-090500.ips")
	before, err := helperCrashReports(dir)
	if err != nil {
		t.Fatal(err)
	}
	clean := failOnHelperCrashes(Evidence{Passed: true}, dir, before)
	if !clean.Passed || len(clean.Assertions) != 0 {
		t.Fatalf("run without a new Helper report changed evidence: %+v", clean)
	}
	write("Safari-2026-10-21-170000.ips")
	write("AppleBridgeHelper-2026-10-21-170042.ips")
	crashed := failOnHelperCrashes(Evidence{Passed: true, Assertions: []Assertion{{Name: "functional", Passed: true}}}, dir, before)
	if crashed.Passed || len(crashed.Assertions) != 2 || crashed.Assertions[1].Name != "helper_crash_free" || crashed.Assertions[1].Passed {
		t.Fatalf("new Helper report did not fail evidence: %+v", crashed)
	}
	if crashed.Assertions[1].Detail != "new Helper crash reports: AppleBridgeHelper-2026-10-21-170042.ips" {
		t.Fatalf("unexpected detail %q", crashed.Assertions[1].Detail)
	}
}

func TestMissingReportsDirectorySkipsCrashCheck(t *testing.T) {
	if _, err := helperCrashReports(filepath.Join(t.TempDir(), "missing")); err == nil {
		t.Fatal("missing directory should report an error so the caller skips the check")
	}
}
