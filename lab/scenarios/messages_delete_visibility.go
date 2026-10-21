package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runNativeDeleteVisibility(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	if level == "real" {
		return runRetainedDeleteTruth(ctx, e)
	}
	output, err := exec.CommandContext(ctx, "packaging/dev/check-messages-capture.sh").CombinedOutput()
	e.Assertions = []Assertion{{Name: "native_capture_visibility_and_restore", Passed: err == nil && strings.Contains(string(output), "all Messages capture checks passed")}}
	for _, line := range strings.Split(string(output), "\n") {
		if strings.HasPrefix(line, "Messages capture checks failed: ") {
			e.Assertions[0].Detail = strings.TrimPrefix(line, "Messages capture checks failed: ")
		}
	}
	e.Passed = e.Assertions[0].Passed
	return e
}

func runRetainedDeleteTruth(ctx context.Context, e Evidence) Evidence {
	run := os.Getenv("APPLE_BRIDGE_DELETE_PROOF_RUN")
	if !regexp.MustCompile(`^[A-Za-z0-9_-]+$`).MatchString(run) {
		e.Status = "environment-limited"
		e.Assertions = []Assertion{{Name: "registered_deleted_test_fixture", Status: "unverified", Detail: "Set APPLE_BRIDGE_DELETE_PROOF_RUN to a cleaned interactive self-test run; this proof never dispatches a write"}}
		return e
	}
	bytes, err := os.ReadFile(filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup", run+".json"))
	var manifest struct {
		RunID     string    `json:"run_id"`
		Scenario  string    `json:"scenario"`
		NativeKey string    `json:"native_key"`
		Text      string    `json:"interactive_text"`
		CreatedAt time.Time `json:"created_at"`
	}
	if err != nil || json.Unmarshal(bytes, &manifest) != nil || manifest.RunID != run || manifest.Scenario != "MSG-INTERACTIVE-GATE" || manifest.NativeKey == "" || manifest.Text != "Apple Bridge interactive self-check "+run {
		e.Assertions = []Assertion{{Name: "registered_deleted_test_fixture", Passed: false}}
		return e
	}
	w, err := world.NewReal(ctx, "real", 0, world.BlobOptions{Scope: "messages:read,system:read"})
	if err != nil {
		e.Assertions = []Assertion{{Name: "real_world", Passed: false}}
		return e
	}
	defer w.Close()
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	proof, err := w.Truth("observe", map[string]any{"source": "messages", "guid": manifest.NativeKey, "expected_text": manifest.Text})
	retained := false
	rows, _ := proof["records"].([]any)
	for _, value := range rows {
		row, _ := value.(map[string]any)
		retained = retained || row["guid"] == manifest.NativeKey && row["is_from_me"] == true && row["visible"] == false && row["expected_text_present"] == true
	}
	add("truth_retains_deleted_test_message", err == nil && proof["status"] == "verified" && retained)
	state, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?mode=ro")
	if err != nil {
		add("state_readonly", false)
		return e
	}
	defer state.Close()
	var id string
	add("retained_target_captured", w.Await(2*time.Minute, func() bool {
		return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", manifest.NativeKey).Scan(&id) == nil
	}) == nil)
	var result contracts.ReadResult
	err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: id}, &result)
	var record struct {
		DeletedAt string `json:"deleted_at"`
	}
	if len(result.Records) == 1 {
		json.Unmarshal(result.Records[0].Data, &record)
	}
	add("copy_retains_recoverable_record", err == nil && len(result.Records) == 1 && !result.Records[0].Deleted && record.DeletedAt != "")
	native, err := sql.Open("sqlite", "file:"+filepath.Join(os.Getenv("HOME"), "Library", "Messages", "chat.db")+"?mode=ro")
	if err == nil {
		var raw float64
		err = native.QueryRowContext(ctx, "SELECT MAX(r.delete_date) FROM chat_recoverable_message_join r JOIN message m ON m.ROWID=r.message_id WHERE m.guid=?", manifest.NativeKey).Scan(&raw)
		native.Close()
		actual, problem := time.Parse(time.RFC3339Nano, record.DeletedAt)
		expected := time.Unix(978307200, int64(raw)).UTC()
		delta := actual.Sub(expected)
		if delta < 0 {
			delta = -delta
		}
		add("native_deleted_timestamp_matches_copy", err == nil && problem == nil && delta < time.Second)
	} else {
		add("native_deleted_timestamp_matches_copy", false)
	}
	info, err := os.Stat(filepath.Join(w.Gateway.Home, "helper", "dispatch-guard.log"))
	add("no_command_dispatched", err == nil && info.Size() == 0)
	e.Metrics = map[string]any{"target_run": run, "writes_submitted": 0, "proof_scope": "retained_native_record_from_previously_verified_test_cleanup"}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
