package scenarios

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
)

func runMessagesCleanupRecovery(ctx context.Context, e Evidence) (result Evidence) {
	result = e
	result.Status = "limited"
	result.Metrics = map[string]any{"writes_submitted": 0, "cleanup_recovery_only": true}
	add := func(name string, passed bool) {
		result.Assertions = append(result.Assertions, Assertion{Name: name, Passed: passed})
	}
	run := os.Getenv("APPLE_BRIDGE_MESSAGE_CLEANUP_RUN")
	key := os.Getenv("APPLE_BRIDGE_MESSAGE_CLEANUP_CACHE")
	if !regexp.MustCompile(`^variant_[A-F0-9]{32}$`).MatchString(run) || !regexp.MustCompile(`^[a-f0-9]{64}$`).MatchString(key) {
		add("explicit_current_run_recovery", false)
		return
	}
	ledger := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup", run+".json")
	data, err := os.ReadFile(ledger)
	digest := sha256.Sum256(data)
	var manifest struct {
		RunID    string `json:"run_id"`
		Scenario string `json:"scenario"`
		Baseline int64  `json:"baseline_max_row_id"`
	}
	if err != nil || hex.EncodeToString(digest[:]) != os.Getenv("APPLE_BRIDGE_MESSAGE_CLEANUP_LEDGER_SHA256") || json.Unmarshal(data, &manifest) != nil || manifest.RunID != run || manifest.Scenario != "MSG-AUDIO-MESSAGE" || manifest.Baseline <= 0 {
		add("unchanged_current_run_ledger", false)
		return
	}
	before, err := cleanupSnapshot(ctx, "messages")
	if err != nil {
		add("recovery_baseline", false)
		return
	}
	db, err := sql.Open("sqlite", "file:"+filepath.Join(os.Getenv("HOME"), "Library", "Messages", "chat.db")+"?mode=ro")
	if err != nil {
		add("recovery_readonly_truth", false)
		return
	}
	defer db.Close()
	tag := "[ab-test " + run + "]"
	rows, err := db.QueryContext(ctx, `SELECT DISTINCT m.guid FROM message m WHERE m.ROWID>? AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0 OR EXISTS(SELECT 1 FROM message_attachment_join j JOIN attachment a ON a.ROWID=j.attachment_id WHERE j.message_id=m.ROWID AND (instr(a.filename,?)>0 OR instr(a.transfer_name,?)>0)))`, manifest.Baseline, tag, tag, "ab-"+run+".caf", "ab-"+run+".caf")
	if err != nil {
		add("recovery_owned_identity_truth", false)
		return
	}
	owned := map[string]bool{}
	for rows.Next() {
		var id string
		if rows.Scan(&id) == nil {
			owned[id] = true
		}
	}
	err = rows.Err()
	rows.Close()
	if err != nil || len(owned) == 0 {
		add("recovery_owned_identity_truth", false)
		return
	}
	baseline := []cleanupItem{}
	for _, item := range before {
		if !owned[strings.SplitN(item.ID, "|", 2)[0]] {
			baseline = append(baseline, item)
		}
	}
	audit, err := newRunCleanup(e.RunID, baseline)
	if err != nil {
		add("recovery_baseline", false)
		return
	}
	defer func() { finishCleanupAudit(context.Background(), audit, "messages", &result) }()
	cache, err := os.UserCacheDir()
	if err != nil {
		add("recovery_signed_helper", false)
		return
	}
	app := filepath.Join(cache, "AppleBridgeDev", "signed-apps", key, "AppleBridgeHelper.app")
	if exec.CommandContext(ctx, "codesign", "--verify", "--strict", app).Run() != nil {
		add("recovery_signed_helper", false)
		return
	}
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "recovery-"+e.RunID+".json")
	defer os.Remove(report)
	err = exec.CommandContext(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", "probe", "cleanup-tagged-self", "--tag", tag, "--owned-only", "--report", report).Run()
	data, readErr := os.ReadFile(report)
	var proof struct {
		Passed    bool   `json:"passed"`
		Deleted   int    `json:"deleted"`
		Remaining int    `json:"remaining_tagged_self_messages"`
		Failure   string `json:"failure"`
	}
	valid := err == nil && readErr == nil && json.Unmarshal(data, &proof) == nil
	add("owned_messages_cleanup", valid && proof.Passed && proof.Remaining == 0)
	result.Metrics["deleted_owned_messages"] = proof.Deleted
	result.Metrics["remaining_owned_messages"] = proof.Remaining
	result.Conditions = map[string]string{"reason": "The CAF attachment did not have the native audio-message flag. This recovery sends no new messages and removes only the explicitly selected run's rows."}
	return
}
