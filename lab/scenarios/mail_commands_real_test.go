package scenarios

import (
	"strings"
	"testing"
)

func TestDecodeMailProof(t *testing.T) {
	proof, err := decodeMailProof([]byte(`{"draft_created":true,"cleanup_passed":true}`))
	if err != nil || !proof["draft_created"] || !proof["cleanup_passed"] {
		t.Fatalf("proof=%v err=%v", proof, err)
	}
	proof, err = decodeMailProof([]byte(`{"fixture_and_commands":false,"cleanup_passed":false,"command_error":"Mail did not answer. Try again.","cleanup_error":"Cleanup did not finish. Try again.","cleanup_error_number":-1728}`))
	if err == nil || !strings.Contains(err.Error(), "Mail did not answer") || !strings.Contains(err.Error(), "Cleanup did not finish") || len(proof) != 2 || proof["fixture_and_commands"] || proof["cleanup_passed"] {
		t.Fatalf("proof=%v err=%v", proof, err)
	}
	for _, data := range []string{`broken`, `{}`, `null`, `{"command_error":true}`, `{"command_error":null}`, `{"cleanup_passed":null}`, `{"cleanup_passed":"true"}`, `{"cleanup_passed":true,"cleanup_error_number":-1728}`, `{"cleanup_error":"Cleanup failed","cleanup_error_number":"wrong"}`} {
		if _, err := decodeMailProof([]byte(data)); err == nil {
			t.Fatalf("accepted invalid proof: %s", data)
		}
	}
}

func TestMailRecoveryRuns(t *testing.T) {
	run := "run_01M6A0RCT483KRP31H95PDZ02F"
	runs, err := mailRecoveryRuns(run + "\n" + run)
	if err != nil || len(runs) != 2 {
		t.Fatalf("runs=%v err=%v", runs, err)
	}
	for _, value := range []string{"foreign", "[ab-test] " + run, run + "'", strings.Repeat(run+" ", 11)} {
		if _, err := mailRecoveryRuns(value); err == nil {
			t.Fatalf("accepted %q", value)
		}
	}
}
