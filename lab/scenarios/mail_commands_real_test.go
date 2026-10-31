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
	proof, err = decodeMailProof([]byte(`{"fixture_and_commands":false,"cleanup_passed":false,"command_error":"Mail did not answer. Try again.","cleanup_error":"Cleanup did not finish. Try again."}`))
	if err == nil || !strings.Contains(err.Error(), "Mail did not answer") || !strings.Contains(err.Error(), "Cleanup did not finish") || len(proof) != 2 || proof["fixture_and_commands"] || proof["cleanup_passed"] {
		t.Fatalf("proof=%v err=%v", proof, err)
	}
	for _, data := range []string{`broken`, `{}`, `null`, `{"command_error":true}`, `{"command_error":null}`, `{"cleanup_passed":null}`, `{"cleanup_passed":"true"}`} {
		if _, err := decodeMailProof([]byte(data)); err == nil {
			t.Fatalf("accepted invalid proof: %s", data)
		}
	}
}
