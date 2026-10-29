package contracts

import (
	"encoding/json"
	"testing"
)

func TestMacActionValidation(t *testing.T) {
	for _, tc := range []struct {
		op, args string
		valid    bool
	}{
		{"mac.notify", `{"title":"bridge-lab","body":"fixture","action_url":"https://example.com"}`, true},
		{"mac.notify", `{"title":""}`, false},
		{"mac.notify", `{"title":"fixture","action_url":"file:///tmp/data"}`, false},
		{"mac.open_settings", `{"pane":"notifications"}`, true},
		{"mac.open_settings", `{"pane":"unknown"}`, false},
		{"mac.set_wallpaper", `{"image_blob":"aW1hZ2U="}`, true},
		{"mac.set_wallpaper", `{"image_blob":"invalid"}`, false},
		{"mac.restart", `{}`, true},
	} {
		c := Command{ID: NewCommandID(), Op: tc.op, Args: json.RawMessage(tc.args)}
		if (ValidateMacCommand(c) == nil) != tc.valid {
			t.Errorf("%s validation mismatch", tc.op)
		}
	}
}
