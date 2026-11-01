package contracts

import (
	"encoding/json"
	"testing"
)

func TestNoteHTMLCompatiblePreservesStructure(t *testing.T) {
	before, _ := json.Marshal(map[string]any{"html": `<div>Title</div><div><b>bold</b> <i>italic</i> <a href="https://example.org/">link</a></div>`, "content_status": "complete", "block_status": "complete", "round_trip_status": "unproven", "attachments": []any{}})
	for _, tt := range []struct {
		html string
		ok   bool
	}{
		{`<div>Title</div><div><b>edited</b> <i>italic</i> <a href="https://example.org/">link</a></div>`, true},
		{`<div>Title</div><div>bold</div>`, false},
		{`<div>Title</div><div><b>bold</b> <i>italic</i> <a href="https://evil.example/">link</a></div>`, false},
		{`<div>Title</div><div><b>bold<script>x</script></b> <i>italic</i> <a href="https://example.org/">link</a></div>`, false},
		{`<div>Title</div><div><b>bold</b> <i>italic</i> <a href="https://example.org/">link</a></div><`, false},
	} {
		if got := NoteHTMLCompatible(before, tt.html); got != tt.ok {
			t.Fatalf("compatible=%v want %v", got, tt.ok)
		}
	}
}

func TestNoteHTMLCommandRequiresExclusiveReplacement(t *testing.T) {
	for _, args := range []string{`{"id":"n","html":"<div>x</div>","plain_text":"x"}`, `{"id":"n","html":"<div>x</div>","mode":"append"}`, `{"id":"n","html":""}`} {
		if _, err := ValidateNoteCommand(Command{ID: NewCommandID(), Op: "notes.update", ExpectedVersion: 1, Args: json.RawMessage(args)}); err == nil {
			t.Fatal("invalid HTML command accepted")
		}
	}
	if _, err := ValidateNoteCommand(Command{ID: NewCommandID(), Op: "notes.update", ExpectedVersion: 1, Args: json.RawMessage(`{"id":"n","html":"<div>x</div>"}`)}); err != nil {
		t.Fatal(err)
	}
}
