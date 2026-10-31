package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCleanupRecoveryRejectsUnboundOrForeignLedgersBeforeNativeAccess(t *testing.T) {
	for _, scenario := range []string{"MSG-AUDIO-MESSAGE", "MSG-SELFTEST"} {
		t.Run(scenario, func(t *testing.T) {
			t.Setenv("HOME", t.TempDir())
			run := "variant_" + strings.Repeat("A", 32)
			t.Setenv("APPLE_BRIDGE_MESSAGE_CLEANUP_RUN", run)
			t.Setenv("APPLE_BRIDGE_MESSAGE_CLEANUP_CACHE", strings.Repeat("a", 64))
			dir := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup")
			if err := os.MkdirAll(dir, 0700); err != nil {
				t.Fatal(err)
			}
			data := []byte(`{"run_id":"` + run + `","scenario":"` + scenario + `","baseline_max_row_id":10}`)
			if err := os.WriteFile(filepath.Join(dir, run+".json"), data, 0600); err != nil {
				t.Fatal(err)
			}
			digest := sha256.Sum256(data)
			if scenario == "MSG-SELFTEST" {
				t.Setenv("APPLE_BRIDGE_MESSAGE_CLEANUP_LEDGER_SHA256", hex.EncodeToString(digest[:]))
			}
			e := runMessagesCleanupRecovery(context.Background(), Evidence{})
			if e.Passed || e.Metrics["writes_submitted"] != 0 || len(e.Assertions) != 1 || e.Assertions[0].Name != "unchanged_current_run_ledger" || e.Assertions[0].Passed {
				t.Fatal(e)
			}
		})
	}
}
