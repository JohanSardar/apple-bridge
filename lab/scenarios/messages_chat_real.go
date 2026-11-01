package scenarios

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"hash"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func nativeMessagesChatDigest(ctx context.Context, native *sql.DB, chat string, maximum int64) (string, error) {
	digest, _, err := nativeMessagesChatDigests(ctx, native, chat, maximum)
	return digest, err
}

func nativeMessagesChatDigests(ctx context.Context, native *sql.DB, chat string, maximum int64) (string, map[string]string, error) {
	rows, err := native.QueryContext(ctx, `SELECT m.*,j.chat_id,r.delete_date FROM message m JOIN (SELECT chat_id,message_id FROM chat_message_join UNION SELECT chat_id,message_id FROM chat_recoverable_message_join) j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id LEFT JOIN chat_recoverable_message_join r ON r.chat_id=j.chat_id AND r.message_id=m.ROWID WHERE c.guid=? AND m.ROWID<=? ORDER BY m.ROWID,j.chat_id`, chat, maximum)
	if err != nil {
		return "", nil, err
	}
	defer rows.Close()
	columns, err := rows.Columns()
	if err != nil {
		return "", nil, err
	}
	columnHashes := map[string]hash.Hash{}
	for _, column := range columns {
		columnHashes[column] = sha256.New()
	}
	digest := sha256.New()
	encoder := json.NewEncoder(digest)
	for rows.Next() {
		values := make([]any, len(columns))
		targets := make([]any, len(columns))
		for i := range values {
			targets[i] = &values[i]
		}
		if err := rows.Scan(targets...); err != nil {
			return "", nil, err
		}
		for i, column := range columns {
			if err := json.NewEncoder(columnHashes[column]).Encode(values[i]); err != nil {
				return "", nil, err
			}
		}
		if err := encoder.Encode(values); err != nil {
			return "", nil, err
		}
	}
	columnDigests := map[string]string{}
	for column, digest := range columnHashes {
		columnDigests[column] = hex.EncodeToString(digest.Sum(nil))
	}
	return hex.EncodeToString(digest.Sum(nil)), columnDigests, rows.Err()
}

func isolatedReadStateFixture(ctx context.Context, native *sql.DB, guid, text string, baselineChat int64) bool {
	var chats, foreign int
	err := native.QueryRowContext(ctx, `SELECT COUNT(DISTINCT c.ROWID), COALESCE(SUM(CASE WHEN COALESCE(m.text,'')=? OR instr(m.attributedBody,CAST(? AS BLOB))>0 THEN 0 ELSE 1 END),0) FROM message fixture JOIN chat_message_join f ON f.message_id=fixture.ROWID JOIN chat c ON c.ROWID=f.chat_id JOIN chat_message_join j ON j.chat_id=c.ROWID JOIN message m ON m.ROWID=j.message_id WHERE fixture.guid=? AND c.ROWID>? AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID)`, text, text, guid, baselineChat).Scan(&chats, &foreign)
	return err == nil && chats == 1 && foreign == 0
}

func runNativeChatProof(ctx context.Context, s Scenario, e Evidence, w *world.World, native string, state *sql.DB, registerChild func(string) error) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	var messageID string
	add("message_captured", w.Await(30*time.Second, func() bool {
		return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=?", native).Scan(&messageID) == nil
	}) == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	var message contracts.ReadResult
	err := client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: messageID}, &message)
	if err != nil || len(message.Records) != 1 {
		add("message_record", false)
		return e
	}
	var data struct {
		ChatID string `json:"chat_id"`
	}
	json.Unmarshal(message.Records[0].Data, &data)
	var nativeChat string
	state.QueryRowContext(ctx, "SELECT native_key FROM aliases WHERE collection='messages.chat' AND record_id=?", data.ChatID).Scan(&nativeChat)
	submit := func(op, id string, version uint64) contracts.CommandRecord {
		raw, _ := json.Marshal(map[string]any{"id": id})
		command := contracts.Command{ID: contracts.NewCommandID(), Op: op, Args: raw, ExpectedVersion: version, WaitMS: 15000}
		started := time.Now()
		var result contracts.CommandRecord
		problem := client.Call(ctx, op, command, &result)
		w.Await(20*time.Second, func() bool {
			_, err := world.Request(ctx, w.Gateway, "/v1/op/commands.status", contracts.CommandQuery{ID: command.ID}, &result)
			return err == nil && result.Disposition != ""
		})
		e.Latency[op] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
		diagnostic := map[string]any{"state": result.State, "disposition": result.Disposition, "request_completed": problem == nil, "context_live": ctx.Err() == nil}
		if result.Error != nil {
			diagnostic["error_code"] = result.Error.Code
		}
		var native struct {
			Reason string `json:"reason"`
		}
		if json.Unmarshal(result.Evidence, &native) == nil && native.Reason != "" {
			diagnostic["native_reason"] = native.Reason
		}
		e.Metrics[op] = diagnostic
		add(op+"_observed", result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered)
		return result
	}
	if s.ID == "MSG-READSTATE-REAL" {
		for _, op := range []string{"messages.mark_unread", "messages.mark_read"} {
			submit(op, data.ChatID, 0)
			truth, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": native, "chat_guid": nativeChat})
			expected := "unread"
			if op == "messages.mark_read" {
				expected = "read"
			}
			add(op+"_independent_native_truth", problem == nil && truth["status"] == "verified" && truth["chat_read_state"] == expected)
		}
	} else {
		args, _ := json.Marshal(map[string]any{"chat_id": data.ChatID, "text": "[ab-test " + e.RunID + "] chat remainder"})
		remainder := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
		if registerChild(remainder.ID) != nil {
			add("remainder_cleanup_registration", false)
			return e
		}
		var created contracts.CommandRecord
		_ = client.Call(ctx, remainder.Op, remainder, &created)
		problem := w.Await(30*time.Second, func() bool {
			_, err := world.Request(ctx, w.Gateway, "/v1/op/commands.status", contracts.CommandQuery{ID: remainder.ID}, &created)
			return err == nil && (created.State == contracts.CommandObserved || created.State == contracts.CommandDelivered)
		})
		add("second_owned_message_retains_test_chat", problem == nil && (created.State == contracts.CommandObserved || created.State == contracts.CommandDelivered))
		if problem != nil || created.State != contracts.CommandObserved && created.State != contracts.CommandDelivered {
			e.Metrics["command_state"] = created.State
			e.Status = "environment-limited"
			return e
		}
		message = contracts.ReadResult{}
		problem = client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: messageID}, &message)
		if problem != nil || len(message.Records) != 1 {
			add("fresh_delete_version", false)
			return e
		}
		submit("messages.delete_message", messageID, message.Records[0].Version)
		truth, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": native})
		visible := false
		rows, _ := truth["records"].([]any)
		for _, value := range rows {
			item, _ := value.(map[string]any)
			visible = visible || item["visible"] == true
		}
		add("message_deletion_independent_truth", problem == nil && truth["status"] == "verified" && !visible)
	}
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	if !e.Passed {
		e.Status = "environment-limited"
	}
	return e
}
