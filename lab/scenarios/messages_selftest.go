package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runNativeMessagesSelfTest(ctx context.Context, e Evidence) (result Evidence) {
	if os.Getenv("APPLE_BRIDGE_MESSAGE_CLEANUP_RUN") != "" {
		return runMessagesCleanupRecovery(ctx, e)
	}
	before, problem := cleanupSnapshot(ctx, "messages")
	if problem != nil {
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "cleanup_baseline", Status: "limited", Detail: "needs Messages Full Disk Access"})
		return e
	}
	owned, problem := newRunCleanup(e.RunID, before)
	if problem != nil {
		return e
	}
	defer func() { finishCleanupAudit(context.Background(), owned, "messages", &result) }()

	dir, err := os.MkdirTemp("", "ab-messages-selftest-")
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "private_helper_directory", Passed: false})
		return e
	}
	defer os.RemoveAll(dir)
	app := filepath.Join(dir, "AppleBridgeHelper.app")
	if err = world.BuildHelper(ctx, app); err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "signed_helper_build", Passed: false})
		return e
	}
	bounded, cancel := context.WithTimeout(ctx, 10*time.Minute)
	defer cancel()
	args := []string{"selftest", "messages", "--helper-app", app, "--force"}
	if os.Getenv("APPLE_BRIDGE_SELFTEST_NO_UI") == "1" {
		args = append(args, "--no-ui")
	}
	data, problem := exec.CommandContext(bounded, "bin/bridge", args...).Output()
	var report struct {
		State   string `json:"state"`
		OSBuild string `json:"os_build"`
		RunID   string `json:"run_id"`
		Steps   []struct {
			Name      string  `json:"name"`
			Operation string  `json:"operation"`
			Proven    bool    `json:"proven"`
			Reason    string  `json:"reason"`
			Latency   float64 `json:"latency_ms"`
		} `json:"steps"`
	}
	err = json.Unmarshal(data, &report)
	e.Metrics = map[string]any{"selftest_run_id": report.RunID, "selftest_state": report.State, "os_build": report.OSBuild, "ui_deferred": os.Getenv("APPLE_BRIDGE_SELFTEST_NO_UI") == "1"}
	e.Assertions = append(e.Assertions, Assertion{Name: "selftest_report", Passed: err == nil && report.RunID != ""})
	proven := map[string]bool{}
	for _, step := range report.Steps {
		proven[step.Name] = step.Proven
		e.Assertions = append(e.Assertions, Assertion{Name: step.Name, Passed: step.Proven, Detail: step.Reason})
		if step.Latency > 0 {
			e.Latency[step.Name] = stats([]float64{step.Latency}, 0)
		}
	}
	for _, name := range []string{"send_text", "send_file", "reply", "love_add", "love_remove", "emoji_add", "emoji_remove", "edit", "unsend", "mark_read", "mark_unread", "delete_message", "cleanup"} {
		e.Assertions = append(e.Assertions, Assertion{Name: "required_" + name, Passed: proven[name]})
	}
	if report.RunID != "" {
		database, problem := sql.Open("sqlite", "file:"+filepath.Join(os.Getenv("HOME"), "Library", "Messages", "chat.db")+"?mode=ro")
		remaining, fileCount := -1, -1
		if problem == nil {
			remaining, fileCount, problem = nativeMessagesSelfTestRemaining(ctx, database, report.RunID)
			database.Close()
		}
		e.Metrics["remaining_text_messages"] = remaining
		e.Metrics["remaining_file_messages"] = fileCount
		e.Assertions = append(e.Assertions, Assertion{Name: "independent_file_cleanup", Passed: problem == nil && fileCount == 0})
		e.Assertions = append(e.Assertions, Assertion{Name: "independent_tagged_cleanup", Passed: problem == nil && remaining == 0})
	}
	e.Passed = problem == nil && err == nil && report.State == "proven"
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	if !e.Passed {
		e.Status = "environment-limited"
	}
	return e
}

func nativeMessagesSelfTestRemaining(ctx context.Context, database *sql.DB, run string) (int, int, error) {
	var texts, files int
	tag := "[ab-test " + run + "]"
	err := database.QueryRowContext(ctx, "SELECT COUNT(DISTINCT m.ROWID) FROM message m JOIN chat_message_join c ON c.message_id=m.ROWID WHERE (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0) AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)", tag, tag).Scan(&texts)
	if err != nil {
		return 0, 0, err
	}
	err = database.QueryRowContext(ctx, "SELECT COUNT(DISTINCT m.ROWID) FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id JOIN chat_message_join c ON c.message_id=m.ROWID WHERE instr(a.filename,?)>0 AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)", "ab-"+run+".").Scan(&files)
	return texts, files, err
}
