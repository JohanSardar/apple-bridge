package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func runAgentLevels(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	create := func(level string) (trust.Agent, string) {
		resp, raw, err := c.request("POST", "agents", map[string]any{"name": "Bridge Test " + level, "level": level, "scopes": []string{"messages:write"}})
		var v struct {
			Agent trust.Agent `json:"agent"`
			Token string      `json:"token"`
		}
		json.Unmarshal(raw, &v)
		add("create_"+level, err == nil && resp.StatusCode == 201 && v.Token != "")
		return v.Agent, v.Token
	}
	add("message_fixture", w.ControlAction("/messages", map[string]any{"action": "seed", "count": 0}) == nil)
	var chats contracts.ReadResult
	err := w.Await(5*time.Second, func() bool {
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.chat"}, &chats)
		return err == nil && len(chats.Records) == 1
	})
	if err != nil {
		add("chat_visible", false)
		return
	}
	conn, err := c.socket()
	if err != nil {
		add("owner_live", false)
		return
	}
	defer conn.CloseNow()
	command := func() contracts.Command {
		raw, _ := json.Marshal(map[string]any{"chat_id": chats.Records[0].ID, "text": "[ab-test " + e.RunID + "] owner-approved agent message"})
		return contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, WaitMS: 15000}
	}
	call := func(token string, cmd contracts.Command) (int, contracts.CommandRecord) {
		status, raw, _ := agentRequest(ctx, c.origin, token, "POST", "/v1/op/messages.send", cmd)
		var r contracts.CommandRecord
		json.Unmarshal(raw, &r)
		return status, r
	}
	status := func(token, id string) contracts.CommandRecord {
		_, raw, _ := agentRequest(ctx, c.origin, token, "POST", "/v1/op/commands.status", contracts.CommandQuery{ID: id})
		var r contracts.CommandRecord
		json.Unmarshal(raw, &r)
		return r
	}
	_, readToken := create("read")
	readCommand := command()
	code, _ := call(readToken, readCommand)
	add("read_command_rejected", code == 401)
	effects, _ := w.Effects()
	add("read_no_effect", effects[readCommand.ID] == 0)
	_, approveToken := create("")
	pending := command()
	code, row := call(approveToken, pending)
	add("default_awaiting_approval", code == 200 && row.State == contracts.CommandAwaitingApproval && row.Disposition == contracts.DispositionAwaitingApproval)
	effects, _ = w.Effects()
	add("approval_no_effect_before_consent", effects[pending.ID] == 0)
	_, retry := call(approveToken, pending)
	add("pending_idempotent", len(retry.History) == 1 && retry.State == row.State)
	checkAgentPendingMCP(ctx, w, approveToken, pending, add)
	other := pending
	other.Args = json.RawMessage(strings.Replace(string(pending.Args), "owner-approved", "changed", 1))
	code, _ = call(approveToken, other)
	add("pending_changed_id_conflicts", code == 409)
	timed, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	var frame map[string]any
	err = wsjson.Read(timed, conn, &frame)
	add("approval_live_event", err == nil && frame["t"] == "approval")
	resp, raw, err := c.request("GET", "approvals", nil)
	add("owner_receives_full_text", err == nil && resp.StatusCode == 200 && bytes.Contains(raw, []byte("owner-approved agent message")) && bytes.Contains(raw, []byte("argument_hash")))
	code, _, _ = agentRequest(ctx, c.origin, approveToken, "GET", "/web/v1/approvals", nil)
	add("agent_cannot_approve", code == 401)
	start := time.Now()
	resp, _, err = c.request("POST", "approvals/"+pending.ID, map[string]string{"decision": "approve"})
	add("owner_approves", err == nil && resp.StatusCode == 200)
	err = w.Await(5*time.Second, func() bool { row = status(approveToken, pending.ID); return row.State == contracts.CommandObserved })
	add("approved_sim_send_observed", err == nil && row.CommitSeq > 0)
	e.Assertions[len(e.Assertions)-1].Detail = "state=" + string(row.State) + " disposition=" + string(row.Disposition)

	if err == nil {
		e.Latency["approval_to_observed"] = stats([]float64{float64(time.Since(start)) / float64(time.Millisecond)}, 0)
	}
	effects, _ = w.Effects()
	add("approved_executes_once", effects[pending.ID] == 1)
	resp, _, err = c.request("POST", "approvals/"+pending.ID, map[string]string{"decision": "approve"})
	effects, _ = w.Effects()
	add("approval_retry_no_redispatch", err == nil && resp.StatusCode == 200 && effects[pending.ID] == 1)
	rejected := command()
	call(approveToken, rejected)
	resp, _, err = c.request("POST", "approvals/"+rejected.ID, map[string]string{"decision": "reject"})
	row = status(approveToken, rejected.ID)
	effects, _ = w.Effects()
	add("owner_reject_final_no_effect", err == nil && resp.StatusCode == 200 && row.State == contracts.CommandRejectedByOwner && row.Disposition == contracts.DispositionRejectedByOwner && effects[rejected.ID] == 0)
	expiring := command()
	call(approveToken, expiring)
	add("advance_approval_clock", os.WriteFile(filepath.Join(w.Gateway.Home, "clock-offset"), []byte("86401000"), 0600) == nil)
	_, row = call(approveToken, expiring)
	add("retry_expires_unanswered_approval", row.State == contracts.CommandExpired)
	row = status(approveToken, expiring.ID)
	effects, _ = w.Effects()
	add("approval_expires_24h", row.State == contracts.CommandExpired && effects[expiring.ID] == 0)
	os.Remove(filepath.Join(w.Gateway.Home, "clock-offset"))
	durable := command()
	durable.ExpiresAt = time.Now().UTC().Add(3 * time.Second)
	call(approveToken, durable)
	add("pending_survives_gateway_restart", w.RestartGateway() == nil && status(approveToken, durable.ID).State == contracts.CommandAwaitingApproval)
	delay := time.Until(durable.ExpiresAt.Add(100 * time.Millisecond))
	if delay > 0 {
		timer := time.NewTimer(delay)
		select {
		case <-ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
	}
	approvedAt := time.Now()
	resp, _, err = c.request("POST", "approvals/"+durable.ID, map[string]string{"decision": "approve"})
	add("execution_expiry_starts_at_approval", err == nil && resp.StatusCode == 200)
	err = w.Await(5*time.Second, func() bool { row = status(approveToken, durable.ID); return row.State == contracts.CommandObserved })
	add("expired_original_request_executes_after_approval", err == nil && row.ExpiresAt.After(approvedAt) && row.ExpiresAt.After(durable.ExpiresAt))
	e.Assertions[len(e.Assertions)-1].Detail = "state=" + string(row.State) + " disposition=" + string(row.Disposition)
	runAgentReplicaBoundary(ctx, w, chats.Records[0].ID, command, add)
	_, fullToken := create("full")
	fullCommand := command()
	code, row = call(fullToken, fullCommand)
	add("full_executes_directly", code == 200 && row.State == contracts.CommandObserved)
	checkAgentLevelActivity(c, map[string]contracts.CommandState{readCommand.ID: contracts.CommandRejected, pending.ID: contracts.CommandObserved, rejected.ID: contracts.CommandRejectedByOwner, expiring.ID: contracts.CommandExpired, durable.ID: contracts.CommandObserved, fullCommand.ID: contracts.CommandObserved}, add)
}
