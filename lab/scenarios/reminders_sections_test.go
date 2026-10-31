package scenarios

import "testing"

func TestRemindersProofCommand(t *testing.T) {
	for scenario, expected := range map[string]string{"REM-SECTIONS": "reminders-sections-proof", "WEB-REM-PARITY": "reminders-parity-proof"} {
		if actual := remindersProofCommand(scenario); actual != expected {
			t.Fatalf("%s selected %s, want %s", scenario, actual, expected)
		}
	}
}
