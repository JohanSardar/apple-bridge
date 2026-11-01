package scenarios

import (
	"encoding/json"
	"testing"
)

func TestRecordedNotesCleanupRequiresProvenFixture(t *testing.T) {
	for _, created := range []bool{false, true} {
		raw, _ := json.Marshal(Evidence{Scenario: "NOTE-EXTRAS", Level: "real", RunID: "notes-extras_owned", Assertions: []Assertion{{Name: "tagged_fixture_created", Passed: created}}})
		calls := 0
		err := notesCleanupRecordedExtras(func(verb string, args map[string]any) (map[string]any, error) {
			calls++
			if args["tag"] != "[ab-test notes-extras_owned]" {
				t.Fatal("cleanup widened run")
			}
			if calls == 1 {
				return map[string]any{"id": "captured"}, nil
			}
			folders := args["folders"].([]map[string]any)
			if len(folders) != 1 || folders[0]["id"] != "captured" || folders[0]["title"] != "Bridge Test [ab-test notes-extras_owned]" || args["created_by_run"] != true {
				t.Fatal("cleanup lost captured identity")
			}
			return map[string]any{"status": "verified"}, nil
		}, raw)
		want := 0
		if created {
			want = 2
		}
		if err != nil || calls != want {
			t.Fatalf("err=%v calls=%d want=%d", err, calls, want)
		}
	}
}
