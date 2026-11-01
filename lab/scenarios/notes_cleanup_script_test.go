package scenarios

import (
	"encoding/json"
	"os"
	"os/exec"
	"reflect"
	"strings"
	"testing"
)

func TestNotesCleanupScriptDeletesOnlyValidatedScopedObjects(t *testing.T) {
	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("Node is unavailable")
	}
	bytes, err := os.ReadFile("../truth/native/Sources/AppleTruth/Notes.swift")
	if err != nil {
		t.Fatal(err)
	}
	_, body, ok := strings.Cut(string(bytes), "#\"\"\"\n")
	if !ok {
		t.Fatal("Notes fixture script missing")
	}
	body, _, ok = strings.Cut(body, "\n        \"\"\"#")
	if !ok {
		t.Fatal("Notes fixture script incomplete")
	}
	for _, test := range []struct {
		name    string
		request map[string]any
		deleted []string
		failed  bool
	}{
		{"note", map[string]any{"action": "cleanup_owned", "id": "note", "folder_id": "a"}, []string{"note"}, false},
		{"note moved into captured trash", map[string]any{"action": "cleanup_owned", "id": "note", "folder_id": "a", "trash_folders": map[string]string{"note": "trash"}}, []string{"note", "note"}, false},
		{"already in captured trash", map[string]any{"action": "cleanup_owned", "id": "note", "folder_id": "a", "current_folder": "trash", "trash_folders": map[string]string{"note": "trash"}}, []string{"note"}, false},
		{"other run in trash", map[string]any{"action": "cleanup_owned", "id": "note", "folder_id": "a", "current_folder": "trash", "note_title": "[ab-test other] note", "trash_folders": map[string]string{"note": "trash"}}, []string{}, true},
		{"wrong note folder", map[string]any{"action": "cleanup_owned", "id": "note", "folder_id": "b"}, []string{}, true},
		{"folder", map[string]any{"action": "cleanup_created_folder", "created_by_run": true, "folder_id": "a", "title": "Bridge Test [ab-test owned] a"}, []string{"a"}, false},
		{"batch", map[string]any{"action": "cleanup_created_folders", "created_by_run": true, "folders": []map[string]any{{"id": "a", "title": "Bridge Test [ab-test owned] a"}, {"id": "b", "title": "Bridge Test [ab-test owned] b"}}}, []string{"a", "b"}, false},
		{"invalid second folder", map[string]any{"action": "cleanup_created_folders", "created_by_run": true, "folders": []map[string]any{{"id": "a", "title": "Bridge Test [ab-test owned] a"}, {"id": "b", "title": "Bridge Test [ab-test owned] wrong"}}}, []string{}, true},
		{"occupied second folder", map[string]any{"action": "cleanup_created_folders", "created_by_run": true, "occupied": "b", "folders": []map[string]any{{"id": "a", "title": "Bridge Test [ab-test owned] a"}, {"id": "b", "title": "Bridge Test [ab-test owned] b"}}}, []string{}, true},
		{"duplicate folder", map[string]any{"action": "cleanup_created_folders", "created_by_run": true, "folders": []map[string]any{{"id": "a", "title": "Bridge Test [ab-test owned] a"}, {"id": "a", "title": "Bridge Test [ab-test owned] a"}}}, []string{}, true},
	} {
		t.Run(test.name, func(t *testing.T) {
			test.request["tag"] = "[ab-test owned]"
			test.request["verb"] = "fixture"
			request, _ := json.Marshal(test.request)
			source, _ := json.Marshal(body)
			script := `
const deleted = [];
const request = ` + string(request) + `;
const occupied = request.occupied;
let currentFolder = request.current_folder || "a";
const titles = {a:'Bridge Test [ab-test owned] a',b:'Bridge Test [ab-test owned] b',trash:'Recently Deleted'};
function folder(id) {
    if (!titles[id]) throw Error('missing folder');
    const notes = () => id === occupied ? [{}] : [];
    notes.whose = query => () => query.id === 'note' && id === currentFolder ? [note] : [];
    return {owned:true, id:() => id, name:() => titles[id], folders:() => [], notes,
        container:() => ({folders:{whose:query => () => titles[query.id] ? [folder(query.id)] : []}})};
}
const note = {owned:true, id:() => 'note', name:() => request.note_title || '[ab-test owned] note', container:() => folder(currentFolder)};
const app = {folders:{byId:folder}, notes:{byId:() => note}, delete:object => {
    if (!object.owned) throw Error('unscoped deletion');
    deleted.push(object.id());
}, move:(object,options) => {
    if (!object.owned || options.to.id() !== 'trash') throw Error('invalid trash move');
    deleted.push(object.id());
    currentFolder = 'trash';
}};
let failed = false;
try { Function('Application','request',` + string(source) + `)(() => app,` + string(request) + `); }
catch (error) { failed = true; }
process.stdout.write(JSON.stringify({deleted, failed}));
`
			output, err := exec.Command(node, "-e", script).CombinedOutput()
			if err != nil {
				t.Fatalf("script model failed: %s: %v", output, err)
			}
			var result struct {
				Deleted []string `json:"deleted"`
				Failed  bool     `json:"failed"`
			}
			if err := json.Unmarshal(output, &result); err != nil {
				t.Fatal(err)
			}
			if !reflect.DeepEqual(result.Deleted, test.deleted) || result.Failed != test.failed {
				t.Fatalf("got %+v; want deletes %v, failed %v", result, test.deleted, test.failed)
			}
		})
	}
}
