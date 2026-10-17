package scenarios

import (
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runNativeMessagesCommands(ctx context.Context, s Scenario, level string, e Evidence) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if s.ID != "MSG-SEND-SELF-TEXT" && s.ID != "MSG-SEND-FILES" {
		e.Status = "pending-unlock"
		e.Assertions = append(e.Assertions, Assertion{Name: "native_accessibility", Status: "unverified", Detail: "Screen capture shows password lock screen; Messages AX target cannot be verified"})
		return e
	}
	replicas := 0
	if level == "replica" {
		replicas = 1
		e.Hosts = append(e.Hosts, "ubuntu")
	}
	w, err := world.NewReal(ctx, level, replicas, world.BlobOptions{Scope: "messages:read,messages:write,system:read"})
	if err != nil {
		add("real_world_start", false)
		return e
	}
	defer w.Close()
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-handles-"+e.RunID+".json")
	defer os.Remove(report)
	probeCtx, cancel := context.WithTimeout(ctx, 40*time.Second)
	err = exec.CommandContext(probeCtx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "probe", "self-handles", "--report", report).Run()
	cancel()
	var own struct {
		Handles []string `json:"handles"`
		State   string   `json:"state"`
	}
	data, readErr := os.ReadFile(report)
	json.Unmarshal(data, &own)
	if err != nil || readErr != nil || len(own.Handles) == 0 {
		e.Status = "environment-limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "self_handles_discovery", Status: "unverified", Detail: "Per-run signed Helper self-handles probe did not return handles; no send attempted"})
		return e
	}
	native, err := sql.Open("sqlite", "file:"+filepath.Join(os.Getenv("HOME"), "Library", "Messages", "chat.db")+"?mode=ro")
	if err != nil {
		add("readonly_truth", false)
		return e
	}
	defer native.Close()
	rawChat := ""
	rows, err := native.QueryContext(ctx, "SELECT c.guid,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT COUNT(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC")
	if err == nil {
		for rows.Next() {
			var guid, handle string
			rows.Scan(&guid, &handle)
			for _, self := range own.Handles {
				if strings.EqualFold(strings.ReplaceAll(handle, " ", ""), strings.ReplaceAll(self, " ", "")) {
					rawChat = guid
					break
				}
			}
			if rawChat != "" {
				break
			}
		}
		rows.Close()
	}
	if rawChat == "" {
		add("existing_self_chat_truth", false)
		return e
	}
	stateDB, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?mode=ro")
	if err != nil {
		add("state_readonly", false)
		return e
	}
	defer stateDB.Close()
	var chatID string
	add("self_chat_captured", w.Await(30*time.Second, func() bool {
		return stateDB.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.chat' AND native_key=? LIMIT 1", rawChat).Scan(&chatID) == nil
	}) == nil)
	if chatID == "" {
		return e
	}
	client := scripted.Client{Endpoint: w.Gateway}
	text := "[ab-test " + e.RunID + "] self command"
	args := map[string]any{"chat_id": chatID, "text": text}
	if s.ID == "MSG-SEND-FILES" {
		args["files"] = []contracts.MessageFile{{Name: "ab-one.txt", Data: base64.StdEncoding.EncodeToString([]byte(text + " one"))}, {Name: "ab-two.txt", Data: base64.StdEncoding.EncodeToString([]byte(text + " two"))}}
	}
	var maximum int64
	native.QueryRowContext(ctx, "SELECT COALESCE(MAX(ROWID),0) FROM message").Scan(&maximum)
	raw, _ := json.Marshal(args)
	c := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	type seen struct {
		at   time.Time
		guid string
	}
	echoes := make(chan seen, 1)
	watchCtx, stopWatch := context.WithCancel(ctx)
	defer stopWatch()
	go func() {
		for watchCtx.Err() == nil {
			var guid string
			err := native.QueryRowContext(watchCtx, "SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? ORDER BY m.ROWID LIMIT 1", maximum, rawChat).Scan(&guid)
			if err == nil {
				echoes <- seen{time.Now(), guid}
				return
			}
			select {
			case <-watchCtx.Done():
				return
			case <-time.After(10 * time.Millisecond):
			}
		}
	}()
	started := time.Now()
	var outcome contracts.CommandRecord
	err = client.Call(ctx, c.Op, c, &outcome)
	if err == nil && (outcome.State == contracts.CommandAdmitted || outcome.State == contracts.CommandPrepared || outcome.State == contracts.CommandDispatched) {
		w.Await(20*time.Second, func() bool {
			client.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID}, &outcome)
			return outcome.State != contracts.CommandAdmitted && outcome.State != contracts.CommandPrepared && outcome.State != contracts.CommandDispatched
		})
	}
	e.Metrics = map[string]any{"command_state": outcome.State, "disposition": outcome.Disposition, "screen": "locked", "send_commands": 1, "parts_requested": 1 + len(argsFiles(args))}
	observed := err == nil && (outcome.State == contracts.CommandObserved || outcome.State == contracts.CommandDelivered) && outcome.CommitSeq > 0
	add("native_send_observed_with_fingerprint_checkpoint", observed)
	if !observed {
		var evidence map[string]any
		json.Unmarshal(outcome.Evidence, &evidence)
		if parts, ok := evidence["parts"].([]any); ok {
			codes := []any{}
			for _, part := range parts {
				if value, ok := part.(map[string]any); ok {
					codes = append(codes, value["apple_event_error"])
				}
			}
			e.Metrics["apple_event_errors"] = codes
		}
		e.Status = "environment-limited"
		return e
	}
	var evidence struct {
		NativeKey string `json:"native_key"`
		Parts     []struct {
			NativeKey   string `json:"native_key"`
			Disposition string `json:"disposition"`
		} `json:"parts"`
		Fingerprint       string `json:"fingerprint"`
		CaptureCheckpoint string `json:"capture_checkpoint"`
	}
	json.Unmarshal(outcome.Evidence, &evidence)
	add("request_hash_bound", evidence.Fingerprint == outcome.RequestHash && evidence.CaptureCheckpoint != "")
	var fromMe, joined int
	err = native.QueryRowContext(ctx, "SELECT is_from_me,(SELECT COUNT(*) FROM chat_message_join j WHERE j.message_id=m.ROWID) FROM message m WHERE guid=?", evidence.NativeKey).Scan(&fromMe, &joined)
	add("truth_outgoing_not_echo", err == nil && fromMe == 1 && joined > 0)
	if s.ID == "MSG-SEND-FILES" {
		add("three_native_part_outcomes", len(evidence.Parts) == 3)
		for _, part := range evidence.Parts {
			var count int
			err = native.QueryRowContext(ctx, "SELECT COUNT(*) FROM message WHERE guid=? AND is_from_me=1 AND ROWID>?", part.NativeKey, maximum).Scan(&count)
			add("truth_native_part", err == nil && count == 1)
		}
	}
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	for i, endpoint := range endpoints {
		visible := w.Await(5*time.Second, func() bool {
			var result contracts.ReadResult
			err := (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.message", Where: map[string]any{"text": text}, Limit: 100}, &result)
			return err == nil && len(result.Records) > 0
		}) == nil
		add("command_visible_"+string(rune('0'+i)), visible)
		e.Latency["command_submit_to_visible_"+string(rune('0'+i))] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	}
	select {
	case echo := <-echoes:
		for i, endpoint := range endpoints {
			visible := w.Await(5*time.Second, func() bool {
				var id string
				if stateDB.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", echo.guid).Scan(&id) != nil {
					return false
				}
				var result contracts.ReadResult
				err := (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: id}, &result)
				return err == nil && len(result.Records) == 1
			}) == nil
			add("incoming_echo_visible_"+string(rune('0'+i)), visible)
			e.Latency["echo_chat_db_appearance_to_visible_"+string(rune('0'+i))] = stats([]float64{float64(time.Since(echo.at)) / float64(time.Millisecond)}, 0)
		}
	case <-time.After(5 * time.Second):
		add("incoming_echo_truth", false)
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
func argsFiles(args map[string]any) []contracts.MessageFile {
	files, _ := args["files"].([]contracts.MessageFile)
	return files
}
