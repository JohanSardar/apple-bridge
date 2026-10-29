package contracts

import (
	"encoding/json"
	"testing"
)

func TestCaptureCommandContracts(t *testing.T) {
	for _, tc := range []struct {
		op, args string
		valid    bool
	}{
		{"capture.screenshot", `{}`, true},
		{"capture.screenshot", `{"display":1,"window":2}`, false},
		{"capture.screenshot", `{"display":0}`, false},
		{"capture.camera_photo", `{"seconds":1}`, false},
		{"capture.record_audio", `{"seconds":60}`, true},
		{"capture.record_audio", `{"seconds":61}`, false},
		{"mac.screen_record", `{"seconds":30}`, true},
		{"mac.screen_record", `{"seconds":31}`, false},
		{"print.document", `{"file":"fixture.pdf","printer":"Office"}`, true},
		{"print.document", `{"file":"fixture.pdf"}`, false},
		{"capture.camera_photo", `{"unknown":true}`, false},
	} {
		_, err := ValidateCommand(Command{ID: NewCommandID(), Op: tc.op, Args: json.RawMessage(tc.args)})
		if (err == nil) != tc.valid {
			t.Errorf("%s validation mismatch", tc.op)
		}
		d, ok := Descriptor(tc.op)
		if !ok || d.Kind != CommandOp {
			t.Errorf("%s missing command descriptor", tc.op)
		}
	}
	if CaptureEvidence(json.RawMessage(`{"observed":true,"size":1,"digest":"bad","prepared":{"request_hash":"hash"}}`), "hash") {
		t.Fatal("invalid digest accepted")
	}
}
