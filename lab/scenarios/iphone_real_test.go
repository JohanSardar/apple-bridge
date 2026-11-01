package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestIPhoneRealRequiresSelectedPhysicalDevice(t *testing.T) {
	t.Setenv("AB_REAL_DEVICE", "")
	evidence := runIPhoneReal(context.Background(), Scenario{ID: "APP-CMD-REAL"}, "real")
	if evidence.Passed || len(evidence.Assertions) != 1 || evidence.Assertions[0].Name != "physical_device_selected" {
		t.Fatal("real handler accepted an unselected phone")
	}
}

func TestIPhoneRealRoutesOnlyHardwareScenarios(t *testing.T) {
	for _, id := range []string{"APP-SHELL-REAL", "APP-PAIR-REAL", "APP-CMD-REAL", "APP-NOTIFY-REAL", "APP-APPROVE-REAL", "APP-LIVE-REAL", "APP-INTENTS-REAL", "FOC-PHONE-SYNC"} {
		if !isIPhoneRealScenario(id, "real") || isIPhoneRealScenario(id, "sim") || len(phoneRequirements(id)) == 0 {
			t.Fatalf("incorrect hardware route for %s", id)
		}
	}
	for _, id := range []string{"APP-BUILD-REAL", "APP-CMD-SIM", "APP-HEALTH-REAL"} {
		if isIPhoneRealScenario(id, "real") {
			t.Fatalf("stole unrelated route %s", id)
		}
	}
}

func TestIPhoneRealCannotWriteMacKeychain(t *testing.T) {
	t.Setenv("AB_REAL_DEVICE", "selected")
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "")
	evidence := runIPhoneReal(context.Background(), Scenario{ID: "APP-PAIR-REAL"}, "real")
	if evidence.Passed || evidence.Assertions[0].Name != "isolated_file_secret_store" {
		t.Fatal("phone fixture allowed implicit Mac keychain storage")
	}
}

func TestRealPhoneGatewayServesSignedStatus(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	authority, err := trust.Open(filepath.Join(t.TempDir(), "trust"))
	if err != nil {
		t.Fatal(err)
	}
	server := httptest.NewTLSServer(realPhoneHandler(authority, iphone.NewDelivery(nil, authority)))
	defer server.Close()
	code, err := authority.PairIPhoneCode(server.URL, false, time.Minute)
	if err != nil {
		t.Fatal(err)
	}
	phone, err := simapple.NewIPhone()
	if err != nil {
		t.Fatal(err)
	}
	status, err := phone.Pair(server.Client(), server.URL, code)
	if err != nil || status != 200 {
		t.Fatalf("pair: %d %v", status, err)
	}
	response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/status", nil)
	if err != nil {
		t.Fatal(err)
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		t.Fatalf("signed status: %d", response.StatusCode)
	}
}

func TestRealPhoneReceiptRefreshesAfterNonFinalCopy(t *testing.T) {
	destination := filepath.Join(t.TempDir(), "result.json")
	if err := os.WriteFile(destination, []byte(`{"run":"stale","done":true}`), 0600); err != nil {
		t.Fatal(err)
	}
	for _, done := range []bool{false, true} {
		copy := func(args ...string) error {
			if _, err := os.Stat(destination); !os.IsNotExist(err) {
				return fmt.Errorf("devicectl will skip an existing destination")
			}
			raw, _ := json.Marshal(phoneReceipt{Run: "fresh", Done: done})
			return os.WriteFile(destination, raw, 0600)
		}
		receipt, err := copyRealPhoneReceipt(copy, "phone", "bundle", "source", destination)
		if err != nil || receipt.Run != "fresh" || receipt.Done != done {
			t.Fatalf("fresh receipt: %+v %v", receipt, err)
		}
	}
}

func TestRealPhoneInstructionsAppendSinglePlainLine(t *testing.T) {
	path := filepath.Join(t.TempDir(), "instructions.txt")
	t.Setenv("AB_IPHONE_AUTOMATION_STEPS", path)
	logPhoneInstruction("tap Allow\non the iPhone")
	logPhoneInstruction("complete Face ID")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	lines := strings.Split(strings.TrimSpace(string(raw)), "\n")
	if len(lines) != 2 || !strings.HasSuffix(lines[0], "tap Allow on the iPhone") || !strings.HasSuffix(lines[1], "complete Face ID") {
		t.Fatalf("unexpected prompt lines: %q", raw)
	}
}

func TestRealPhoneIntentCleanupInstructionsMatchPhase(t *testing.T) {
	first := phonePhaseInstruction("APP-INTENTS-REAL", 1)
	cleanup := phonePhaseInstruction("APP-INTENTS-REAL", 2)
	if !strings.Contains(first, "tap +") || !strings.Contains(cleanup, "Delete Shortcut") || strings.Contains(cleanup, "name it") {
		t.Fatal("shortcut cleanup repeated the creation instructions")
	}
}
