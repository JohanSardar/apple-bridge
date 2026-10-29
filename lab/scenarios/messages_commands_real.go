package scenarios

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runNativeMessagesCommands(ctx context.Context, s Scenario, level string, e Evidence) Evidence {
	if s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-UPLOAD-NATIVE-STAGING" {
		e.Status = "limited"
		e.Metrics = map[string]any{"writes_submitted": 0}
		e.Assertions = append(e.Assertions, Assertion{Name: "owned_attachment_cleanup_qualified", Status: "limited", Detail: "needs native attachment cleanup qualification; the captured-ID media target remains unavailable; no send attempted"})
		return e
	}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	replicas := 0
	if level == "replica" || s.ID == "WEB-MEDIA-VARIANTS" {
		replicas = 1
	}
	if level == "replica" {
		e.Hosts = append(e.Hosts, "ubuntu")
	}
	w, err := world.NewReal(ctx, level, replicas, world.BlobOptions{Scope: "messages:read,messages:write,system:read"})
	if err != nil {
		os.WriteFile("/tmp/ablab/messages-real-world-error.log", []byte(err.Error()), 0600)
		add("real_world_start", false)
		return e
	}
	defer w.Close()
	return runNativeMessagesCommandsInWorld(ctx, s, level, e, w)
}

func runNativeMessagesCommandsInWorld(ctx context.Context, s Scenario, level string, e Evidence, w *world.World) (result Evidence) {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-handles-"+e.RunID+".json")
	defer os.Remove(report)
	probeCtx, cancel := context.WithTimeout(ctx, 40*time.Second)
	err := exec.CommandContext(probeCtx, "open", "-n", "-W", "-g", "-a", w.HelperApp, "--args", "probe", "self-handles", "--report", report).Run()
	cancel()
	var own struct {
		Handles              []string `json:"handles"`
		State                string   `json:"state"`
		CleanupAccessibility bool     `json:"cleanup_accessibility"`
	}
	data, readErr := os.ReadFile(report)
	json.Unmarshal(data, &own)
	if err != nil || readErr != nil || len(own.Handles) == 0 {
		e.Status = "environment-limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "self_handles_discovery", Status: "unverified", Detail: "Per-run signed Helper self-handles probe did not return handles; no send attempted"})
		return e
	}
	if (s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-UPLOAD-NATIVE-STAGING") && !own.CleanupAccessibility {
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "cleanup_accessibility", Status: "limited", Detail: "needs Messages Accessibility grant"})
		return e
	}
	if strings.HasPrefix(s.ID, "MSG-GROUP-") || s.ID == "MSG-SEND-RCS-GROUP" {
		return runNativeMessagesGroup(ctx, s, e, w, own.Handles)
	}
	native, err := sql.Open("sqlite", "file:"+filepath.Join(os.Getenv("HOME"), "Library", "Messages", "chat.db")+"?mode=ro")
	if err != nil {
		add("readonly_truth", false)
		return e
	}
	defer native.Close()
	rawChat := ""
	failedNative := ""
	if s.ID == "MSG-RESEND" {
		failedNative, rawChat, err = failedSelfFixture(ctx, native, own.Handles)
		if err != nil {
			e.Status = "environment-limited"
			e.Assertions = append(e.Assertions, Assertion{Name: "unique_failed_test_self_message", Status: "unverified", Detail: "Supplied run has no uniquely verified active failed self message"})
			return e
		}
	}
	newHandle := ""
	chatProof := s.ID == "MSG-READSTATE-REAL" || s.ID == "MSG-DELETE-REAL"
	var maximumChat int64
	if chatProof {
		known := map[string]bool{}
		rows, problem := native.QueryContext(ctx, "SELECT h.id FROM chat_handle_join j JOIN handle h ON h.ROWID=j.handle_id")
		if problem != nil {
			add("existing_routes_truth", false)
			return e
		}
		for rows.Next() {
			var handle string
			if rows.Scan(&handle) != nil {
				rows.Close()
				add("existing_routes_truth", false)
				return e
			}
			known[contracts.NormalizeContactHandle(handle)] = true
		}
		problem = rows.Err()
		rows.Close()
		if problem != nil {
			add("existing_routes_truth", false)
			return e
		}
		for _, handle := range own.Handles {
			if !known[contracts.NormalizeContactHandle(handle)] {
				newHandle = handle
				break
			}
		}
		if newHandle == "" {
			e.Status = "environment-limited"
			e.Assertions = append(e.Assertions, Assertion{Name: "isolated_test_self_chat", Status: "unverified", Detail: "Every discovered own handle already has a chat; existing personal chats were left unchanged"})
			return e
		}
		if native.QueryRowContext(ctx, "SELECT COALESCE(MAX(ROWID),0) FROM chat").Scan(&maximumChat) != nil {
			add("chat_baseline", false)
			return e
		}
	}
	rows, err := native.QueryContext(ctx, "SELECT c.guid,h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.service_name='iMessage' AND (SELECT COUNT(*) FROM chat_handle_join x WHERE x.chat_id=c.ROWID)=1 ORDER BY c.ROWID DESC")
	if err == nil {
		for rawChat == "" && newHandle == "" && rows.Next() {
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
	if rawChat == "" && newHandle == "" {
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
	add("self_chat_captured", newHandle != "" || w.Await(30*time.Second, func() bool {
		return stateDB.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.chat' AND native_key=? LIMIT 1", rawChat).Scan(&chatID) == nil
	}) == nil)
	if chatID == "" && newHandle == "" {
		return e
	}
	client := scripted.Client{Endpoint: w.Gateway}
	text := "[ab-test " + e.RunID + "] self command"
	if s.ID == "MSG-INTERACTIVE-GATE" {
		text = "Apple Bridge interactive self-check " + e.RunID
	}
	args := map[string]any{"chat_id": chatID, "text": text}
	if failedNative != "" {
		var failedID string
		if w.Await(30*time.Second, func() bool {
			return stateDB.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", failedNative).Scan(&failedID) == nil
		}) != nil {
			add("failed_fixture_captured", false)
			return e
		}
		args["resend_of"] = failedID
	}
	if newHandle != "" {
		args = map[string]any{"handles": []string{newHandle}, "service": "iMessage", "text": text}
	}
	if s.ID == "MSG-REPLY" {
		contacts, err := nativeTestContacts()
		handle := contacts["REMY_IMESSAGE_TEST_HANDLE"]
		if handle == "" {
			handle = contacts["REMY_IMESSAGE_TEST_EMAIL"]
		}
		if err != nil || handle == "" {
			add("approved_reply_contact", false)
			return e
		}
		text = "[ab-test " + e.RunID + "] This is an automated test of Johan's Apple Bridge and can be ignored."
		args = map[string]any{"handles": []string{handle}, "service": "iMessage", "text": text}
	}

	if s.ID == "WEB-MEDIA-VARIANTS" {
		files, problem := prepareMediaFiles(ctx, w, e.RunID)
		add("synthetic_heic_mov_caf_staged", problem == nil)
		if problem != nil {
			return e
		}
		args["files"] = files
	}
	if s.ID == "MSG-SEND-FILES" {
		large, problem := stageMessageFile(ctx, w.Gateway, "ab-"+e.RunID+"-one.txt", messageSyntheticFile(text))
		add("five_mib_upload_staged", problem == nil)
		if problem != nil {
			return e
		}
		args["files"] = []contracts.MessageFile{large, {Name: "ab-" + e.RunID + "-two.txt", Data: base64.StdEncoding.EncodeToString([]byte(text + " two"))}}
	}
	var nativeUploadClient *webClient
	if s.ID == "WEB-UPLOAD-NATIVE-STAGING" {
		file, owner, problem := stageNativeWebUpload(ctx, w, e.RunID, messageSyntheticFile(text))
		add("web_upload_verified_ready", problem == nil)
		if problem != nil {
			return e
		}
		args["files"] = []contracts.MessageFile{file}
		nativeUploadClient = owner
	}
	beforeCleanup, cleanupProblem := cleanupSnapshot(ctx, "messages")
	if cleanupProblem != nil {
		add("cleanup_baseline", false)
		return e
	}
	ownedCleanup, cleanupProblem := newRunCleanup(e.RunID, beforeCleanup)
	if cleanupProblem != nil {
		add("cleanup_baseline", false)
		return e
	}
	defer func() { finishCleanupAudit(context.Background(), ownedCleanup, "messages", &result) }()
	var maximum int64
	native.QueryRowContext(ctx, "SELECT COALESCE(MAX(ROWID),0) FROM message").Scan(&maximum)
	raw, _ := json.Marshal(args)
	c := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	cleanupDirectory := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup")
	if os.MkdirAll(cleanupDirectory, 0700) != nil {
		add("cleanup_registered_before_submit", false)
		return e
	}
	cleanupPath := filepath.Join(cleanupDirectory, e.RunID+".json")
	cleanup := map[string]any{"run_id": e.RunID, "command_id": c.ID, "baseline_max_row_id": maximum, "created_at": time.Now().UTC(), "cleanup_state": "pending_native_cleanup", "scenario": s.ID, "level": level}
	if chatProof {
		cleanup["baseline_max_chat_row_id"] = maximumChat
	}
	if s.ID == "MSG-INTERACTIVE-GATE" {
		cleanup["interactive_text"] = text
	}
	if s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-UPLOAD-NATIVE-STAGING" {
		names := []string{}
		for _, file := range argsFiles(args) {
			names = append(names, file.Name)
		}
		cleanup["media_attachment_names"] = names
	}
	saveCleanup := func() error {
		data, err := json.Marshal(cleanup)
		if err != nil {
			return err
		}
		return os.WriteFile(cleanupPath, data, 0600)
	}
	if saveCleanup() != nil {
		add("cleanup_registered_before_submit", false)
		return e
	}
	add("cleanup_registered_before_submit", true)
	creationCtx, stopCreationCapture := context.WithCancel(ctx)
	capturedCreations := captureMessageCreations(creationCtx, native, maximum, e.RunID, cleanupPath+".creations.json")
	defer stopCreationCapture()
	defer func() {
		cleanup["created_message_ids"] = capturedCreations()
		if saveCleanup() != nil {
			result.Assertions = append(result.Assertions, Assertion{Name: "created_ids_persisted", Passed: false})
			return
		}

		ok, metrics := cleanupNativeMessageRun(w, e.RunID, false, s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-UPLOAD-NATIVE-STAGING")
		if s.ID == "MSG-INTERACTIVE-GATE" && !ok {
			result.Status = "environment-limited"
		}
		result.Assertions = append(result.Assertions, Assertion{Name: "tagged_messages_cleanup", Passed: ok})
		if s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-UPLOAD-NATIVE-STAGING" {
			var remaining int
			problem := native.QueryRowContext(context.Background(), "SELECT COUNT(DISTINCT m.ROWID) FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id JOIN chat_message_join c ON c.message_id=m.ROWID WHERE m.ROWID>? AND a.filename LIKE ? AND NOT EXISTS (SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.chat_id)", maximum, "%ab-"+e.RunID+".%").Scan(&remaining)
			result.Assertions = append(result.Assertions, Assertion{Name: "tagged_media_cleanup_readonly_truth", Passed: problem == nil && remaining == 0})
		}
		if result.Metrics == nil {
			result.Metrics = map[string]any{}
		}
		result.Metrics["cleanup"] = metrics
		result.Passed = len(result.Assertions) > 0 && result.Status == ""
		for _, assertion := range result.Assertions {
			result.Passed = result.Passed && assertion.Passed
		}
		cleanup["cleanup_state"] = "cleanup_failed"
		if ok {
			cleanup["cleanup_state"] = "verified_clean"
			if nativeMessageCleanupPending(result.Metrics["command_state"]) || result.Metrics["disposition"] == contracts.DispositionIndeterminate {
				cleanup["cleanup_state"] = "pending_reconciliation"
				cleanup["late_effect_possible"] = true
			}
		}
		cleanup["cleanup"] = metrics
		saveCleanup()
	}()

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
			err := native.QueryRowContext(watchCtx, "SELECT m.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=0 AND c.guid=? AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0) ORDER BY m.ROWID LIMIT 1", maximum, rawChat, text, text).Scan(&guid)
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
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	observers := []*watcher{}
	for _, endpoint := range endpoints {
		observer, err := watch(ctx, endpoint, false, func(contracts.Record) bool { return true })
		if err != nil {
			add("sse_observer_ready", false)
			return e
		}
		observers = append(observers, observer)
		defer observer.close()
	}
	mediaCtx, stopMedia := context.WithCancel(ctx)
	defer stopMedia()
	var mediaAppeared func(string) time.Time
	if s.ID == "WEB-MEDIA-VARIANTS" {
		mediaAppeared = observeMediaAppearance(mediaCtx, native, e.RunID)
	}
	started := time.Now()
	var outcome contracts.CommandRecord
	if nativeUploadClient != nil {
		socket, problem := nativeUploadClient.socket()
		add("web_command_channel", problem == nil)
		if problem != nil {
			return e
		}
		defer socket.CloseNow()
		outcome, err = uploadCommandReply(ctx, socket, c)
	} else if s.ID == "MSG-INTERACTIVE-GATE" {
		var rejected contracts.CommandRecord
		problem := client.Call(ctx, c.Op, c, &rejected)
		_, lookupErr := world.Request(ctx, w.Gateway, "/v1/op/commands.status", contracts.CommandQuery{ID: c.ID}, &rejected)
		dispatched := false
		for _, event := range rejected.History {
			dispatched = dispatched || event.State == contracts.CommandDispatched
		}
		add("lab_untagged_rejected_before_dispatch", problem != nil && lookupErr == nil && rejected.Disposition == contracts.DispositionRejectedBeforeDispatch && !dispatched)
		c.ID = contracts.NewCommandID()
		cleanup["command_id"] = c.ID
		saveCleanup()
		authority, problem := trust.Open(w.Gateway.Home)
		if problem != nil {
			add("device_authority", false)
			return e
		}
		token, _, problem := authority.CreateDeviceSession("trusted")
		if problem != nil {
			add("device_session", false)
			return e
		}
		owner := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
		conn, problem := owner.socket()
		if problem != nil {
			add("interactive_socket", false)
			return e
		}
		defer conn.CloseNow()
		outcome, err = submitInteractiveMessage(ctx, conn, c)
	} else {
		err = client.Call(ctx, c.Op, c, &outcome)
	}
	if err == nil && (outcome.State == contracts.CommandAdmitted || outcome.State == contracts.CommandPrepared || outcome.State == contracts.CommandDispatched) {
		w.Await(20*time.Second, func() bool {
			client.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID}, &outcome)
			return outcome.State != contracts.CommandAdmitted && outcome.State != contracts.CommandPrepared && outcome.State != contracts.CommandDispatched
		})
	}
	initialState := outcome.State
	if err == nil && outcome.State == contracts.CommandIndeterminate {
		err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID, Resolve: true}, &outcome)
	}
	e.Metrics = map[string]any{"initial_command_state": initialState, "command_state": outcome.State, "disposition": outcome.Disposition, "send_commands": 1, "parts_requested": 1 + len(argsFiles(args))}
	if s.ID == "MSG-SEND-FILES" {
		e.Metrics["largest_file_bytes"] = 5 * 1024 * 1024
		e.Metrics["ipc_file_transport"] = "private_path_and_digest"
	}
	observed := err == nil && (outcome.State == contracts.CommandObserved || outcome.State == contracts.CommandDelivered) && outcome.CommitSeq > 0
	add("native_send_observed_with_fingerprint_checkpoint", observed)
	if !observed {
		var evidence map[string]any
		json.Unmarshal(outcome.Evidence, &evidence)
		cleanup["native_parts"] = evidence["parts"]
		cleanup["native_key"] = evidence["native_key"]
		cleanup["outcome_evidence"] = evidence
		saveCleanup()
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
	if chatProof {
		var rowID int64
		problem := native.QueryRowContext(ctx, "SELECT c.guid,c.ROWID FROM chat c JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?", evidence.NativeKey).Scan(&rawChat, &rowID)
		cleanup["created_chat_key"] = rawChat
		cleanup["created_by_run"] = problem == nil && rowID > maximumChat
		saveCleanup()
		if problem != nil || rowID <= maximumChat {
			add("new_self_chat_creation_verified", false)
			return e
		}
	}
	cleanup["native_parts"] = evidence.Parts
	cleanup["native_key"] = evidence.NativeKey
	saveCleanup()
	if s.ID == "WEB-MEDIA-VARIANTS" {
		return verifyMediaVariants(ctx, e, w, "ab-"+e.RunID, mediaAppeared, observers)
	}
	if s.ID == "WEB-UPLOAD-NATIVE-STAGING" {
		add("two_native_part_outcomes", len(evidence.Parts) == 2)
		keys := []string{}
		for _, part := range evidence.Parts {
			var count int
			problem := native.QueryRowContext(ctx, "SELECT COUNT(*) FROM message WHERE guid=? AND is_from_me=1 AND ROWID>?", part.NativeKey, maximum).Scan(&count)
			add("truth_native_part_once", problem == nil && count == 1)
			keys = append(keys, part.NativeKey)
		}
		verifyNativeUploadTransfer(ctx, w, native, keys, messageSyntheticFile(text), e.Metrics, add)
		var sentFiles, sentTexts int
		fileErr := native.QueryRowContext(ctx, "SELECT COUNT(DISTINCT m.ROWID) FROM message m JOIN message_attachment_join j ON j.message_id=m.ROWID JOIN attachment a ON a.ROWID=j.attachment_id WHERE m.ROWID>? AND m.is_from_me=1 AND instr(a.filename,?)>0", maximum, "ab-"+e.RunID+".txt").Scan(&sentFiles)
		textErr := native.QueryRowContext(ctx, "SELECT COUNT(*) FROM message m WHERE m.ROWID>? AND m.is_from_me=1 AND (m.text=? OR instr(m.attributedBody,CAST(? AS BLOB))>0)", maximum, text, text).Scan(&sentTexts)
		add("native_attachment_effect_exactly_one", fileErr == nil && sentFiles == 1)
		add("native_tagged_text_effect_exactly_one", textErr == nil && sentTexts == 1)
		add("verified_upload_reference_persisted", len(outcome.UploadReferences) == 1)
		return e
	}
	if s.ID != "MSG-SEND-SELF-TEXT" && s.ID != "MSG-SEND-FILES" && s.ID != "MSG-INTERACTIVE-GATE" && s.ID != "MSG-RESEND" {
		return runNativeMessageAction(ctx, s, e, w, evidence.NativeKey, text, stateDB, func(id string) error {
			ids, _ := cleanup["additional_command_ids"].([]string)
			cleanup["additional_command_ids"] = append(ids, id)
			return saveCleanup()
		})
	}

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
	var messageID string
	stateDB.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=? LIMIT 1", evidence.NativeKey).Scan(&messageID)
	for i, endpoint := range endpoints {
		visible := w.Await(5*time.Second, func() bool {
			var result contracts.ReadResult
			err := (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: messageID, Limit: 1}, &result)
			return err == nil && len(result.Records) > 0
		}) == nil
		add("command_visible_"+string(rune('0'+i)), visible)
		at, ok := messagesObservedAt(observers[i], text, true)
		add("command_sse_timestamp_"+string(rune('0'+i)), ok)
		if ok {
			e.Latency["command_submit_to_visible_"+string(rune('0'+i))] = stats([]float64{float64(at.Sub(started)) / float64(time.Millisecond)}, 0)
		}
	}
	select {
	case echo := <-echoes:
		cleanup["incoming_echo_native_key"] = echo.guid
		saveCleanup()
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
			at, ok := messagesObservedAt(observers[i], text, false)
			add("incoming_echo_sse_timestamp_"+string(rune('0'+i)), ok)
			if ok {
				e.Latency["echo_chat_db_appearance_to_visible_"+string(rune('0'+i))] = stats([]float64{max(0, float64(at.Sub(echo.at))/float64(time.Millisecond))}, 0)
			}
		}
	case <-time.After(5 * time.Second):
		add("incoming_echo_truth", false)
	}
	if s.ID == "MSG-SEND-FILES" {
		for _, part := range evidence.Parts {
			var guid, name string
			err := native.QueryRowContext(ctx, "SELECT a.guid,a.filename FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.guid=?", part.NativeKey).Scan(&guid, &name)
			if err != nil {
				continue
			}
			expected := messageSyntheticFile(text)
			if strings.Contains(filepath.Base(name), "two") {
				expected = []byte(text + " two")
			}
			var id string
			if stateDB.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.attachment' AND native_key=? LIMIT 1", guid).Scan(&id) != nil {
				add("attachment_alias", false)
				continue
			}
			for i, endpoint := range endpoints {
				diagnostic := map[string]any{"endpoint_index": i}
				add("authorized_replica_attachment_bytes", messagesAttachmentBytes(ctx, w, endpoint, id, expected, diagnostic))
				e.Metrics["attachment_downloads"] = appendDiagnostic(e.Metrics["attachment_downloads"], diagnostic)
			}
		}
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

func messagesObservedAt(observer *watcher, text string, fromMe bool) (time.Time, bool) {
	observer.mu.Lock()
	defer observer.mu.Unlock()
	var earliest time.Time
	for _, seen := range observer.changes {
		if seen.Change.Record.Collection != "messages.message" {
			continue
		}
		var data struct {
			Text   string `json:"text"`
			FromMe bool   `json:"is_from_me"`
		}
		json.Unmarshal(seen.Change.Record.Data, &data)
		if data.Text == text && data.FromMe == fromMe && (earliest.IsZero() || seen.At.Before(earliest)) {
			earliest = seen.At
		}
	}
	return earliest, !earliest.IsZero()
}
func saveIncomingEchoEvidence(e Evidence) error {
	if e.Scenario != "MSG-SEND-SELF-TEXT" || (e.Level != "real" && e.Level != "replica") {
		return nil
	}
	incoming := e
	incoming.Scenario = "MSG-INCOMING"
	incoming.Assertions = []Assertion{}
	incoming.Latency = map[string]Latency{}
	incoming.Metrics = map[string]any{"driven_by": "self_send_incoming_echo", "source_scenario": "MSG-SEND-SELF-TEXT", "maximum_sends": 1, "chat_db_poll_interval_ms": 10}
	for _, a := range e.Assertions {
		if strings.HasPrefix(a.Name, "incoming_echo_") {
			incoming.Assertions = append(incoming.Assertions, a)
		}
	}
	for name, value := range e.Latency {
		if strings.HasPrefix(name, "echo_chat_db_") {
			incoming.Latency[name] = value
		}
	}
	incoming.Passed = len(incoming.Assertions) > 0 && len(incoming.Latency) > 0 && incoming.Status == ""
	for _, a := range incoming.Assertions {
		incoming.Passed = incoming.Passed && a.Passed
	}
	return Save(incoming)
}

func messagesAttachmentBytes(ctx context.Context, w *world.World, endpoint world.Endpoint, id string, expected []byte, diagnostics ...map[string]any) bool {
	diagnostic := map[string]any{}
	if len(diagnostics) > 0 {
		diagnostic = diagnostics[0]
	}
	return w.Await(10*time.Second, func() bool {
		var result contracts.ReadResult
		err := (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.attachment", ID: id}, &result)
		if err != nil || len(result.Records) != 1 {
			return false
		}
		var data struct {
			Blob struct {
				Digest string `json:"digest"`
				State  string `json:"state"`
			} `json:"blob"`
		}
		if json.Unmarshal(result.Records[0].Data, &data) != nil || data.Blob.Digest == "" {
			return false
		}
		status, bytes, err := blobBytes(ctx, endpoint, data.Blob.Digest, id)
		diagnostic["http_status"] = status
		diagnostic["record_blob_state"] = data.Blob.State
		if progress, problem := blobProgress(ctx, endpoint); problem == nil {
			entry := progress.Entries[data.Blob.Digest]
			diagnostic["local_blob_state"] = entry.State
			diagnostic["local_blob_reason"] = entry.Reason
		}
		return err == nil && status == 200 && string(bytes) == string(expected)
	}) == nil
}

func appendDiagnostic(previous any, next map[string]any) []map[string]any {
	values, _ := previous.([]map[string]any)
	return append(values, next)
}

func messageSyntheticFile(tag string) []byte {
	const size = 5 * 1024 * 1024
	part := []byte(tag + " synthetic attachment\n")
	return bytes.Repeat(part, size/len(part)+1)[:size]
}
func stageMessageFile(ctx context.Context, endpoint world.Endpoint, name string, data []byte) (contracts.MessageFile, error) {
	req, err := http.NewRequestWithContext(ctx, "POST", endpoint.URL+"/v1/blobs", bytes.NewReader(data))
	if err != nil {
		return contracts.MessageFile{}, err
	}
	req.Host = endpoint.Host
	req.Header.Set("Authorization", "Bearer "+endpoint.Token)
	response, err := world.HTTP.Do(req)
	if err != nil {
		return contracts.MessageFile{}, err
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		io.Copy(io.Discard, response.Body)
		return contracts.MessageFile{}, contracts.ErrUnavailable
	}
	var upload struct {
		UploadID string `json:"upload_id"`
	}
	err = json.NewDecoder(response.Body).Decode(&upload)
	if upload.UploadID == "" {
		return contracts.MessageFile{}, contracts.ErrInvalid
	}
	return contracts.MessageFile{Name: name, UploadID: upload.UploadID}, err
}
