package contracts

import (
	"encoding/json"
	"testing"
)

func TestFileCommandBoundary(t *testing.T) {
	for _, path := range []string{"../outside", "sub/../../outside", "/outside", "sub/./file"} {
		args, _ := json.Marshal(FileArgs{Root: "/opt/in", Path: path})
		if ValidateFileCommand(Command{ID: "cmd-test", Op: "files.trash", Args: args}) == nil {
			t.Fatal("unsafe path admitted")
		}
	}
	args, _ := json.Marshal(FileArgs{Root: "/opt/in", Path: "file"})
	c := Command{ID: "cmd-test", Op: "files.download", Args: args}
	for _, raw := range []string{`{"observed":true,"fingerprint":"hash","path":"/outside","downloaded":true}`, `{"observed":true,"fingerprint":"hash","path":"/opt/in/file","downloaded":false}`, `{"observed":true,"fingerprint":"other","path":"/opt/in/file","downloaded":true}`} {
		if FileCompletionEvidence(c, json.RawMessage(raw), "hash") {
			t.Fatal("invalid readback accepted")
		}
	}
	if !FileCompletionEvidence(c, json.RawMessage(`{"observed":true,"fingerprint":"hash","path":"/opt/in/file","downloaded":true}`), "hash") {
		t.Fatal("matching readback rejected")
	}
}
