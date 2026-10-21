package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"os/exec"
	"path/filepath"
	"strings"
)

func runAgentActivity(ctx context.Context, w *world.World, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("activity_authority", false)
		return
	}
	agent, token, err := authority.CreateAgent("Bridge Test activity", trust.AgentRead, []contracts.Grant{"messages:write"})
	if err != nil {
		add("activity_agent", false)
		return
	}
	session, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		add("activity_owner", false)
		return
	}
	owner := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: session}}
	principal := w.Gateway
	principal.Token = token
	var out map[string]any
	for i := 0; i < 3; i++ {
		code, err := world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &out)
		add("activity_read", err == nil && code == 200)
	}
	ids := []string{contracts.NewCommandID(), contracts.NewCommandID()}
	text := "[ab-test activity] private synthetic text"
	for _, id := range ids {
		code, _ := world.Request(ctx, principal, "/v1/op/messages.send", map[string]any{"command_id": id, "args": map[string]any{"chat_id": "BridgeTest", "text": text}}, &out)
		effects, err := w.Effects()
		add("activity_denied_without_effect", code == 401 && err == nil && effects[id] == 0)
	}
	resp, raw, err := owner.request("GET", "activity?principal="+url.QueryEscape(agent.ID), nil)
	add("activity_available", err == nil && resp.StatusCode == 200)
	if err != nil || resp.StatusCode != 200 {
		return
	}
	var log struct {
		Rows []struct {
			ID        string `json:"id"`
			Kind      string `json:"kind"`
			Operation string `json:"operation"`
			Count     int    `json:"count"`
			Hash      string `json:"argument_hash"`
		} `json:"activity"`
	}
	json.Unmarshal(raw, &log)
	commands := map[string]bool{}
	reads := 0
	readBuckets := 0
	for _, row := range log.Rows {
		if row.Kind == "read" && row.Operation == "messages.chats" {
			reads += row.Count
			readBuckets++
		}
		if row.Kind == "command" {
			commands[row.ID] = row.Count == 1 && row.Hash != ""
		}
	}
	add("activity_reads_aggregate_per_minute", reads == 3 && readBuckets <= 2)
	add("activity_each_command_individually", commands[agent.ID+":"+ids[0]] && commands[agent.ID+":"+ids[1]])
	add("activity_arguments_redacted", !bytes.Contains(raw, []byte(text)) && !bytes.Contains(raw, []byte(token)))
	resp, filtered, err := owner.request("GET", "activity?principal="+url.QueryEscape(agent.ID)+"&operation=messages.send", nil)
	add("activity_operation_filter", err == nil && resp.StatusCode == 200 && bytes.Contains(filtered, []byte(ids[0])) && !bytes.Contains(filtered, []byte("messages.chats")))
	add("activity_restart", w.RestartGateway() == nil)
	resp, restarted, err := owner.request("GET", "activity?principal="+url.QueryEscape(agent.ID), nil)
	add("activity_survives_restart", err == nil && resp.StatusCode == 200 && bytes.Contains(restarted, []byte(ids[0])))
	_, err = exec.CommandContext(ctx, "sqlite3", filepath.Join(w.Gateway.Home, "state.db"), "UPDATE agent_activity SET at=unixepoch()-91*86400 WHERE principal='"+agent.ID+"'").Output()
	resp, pruned, pruneErr := owner.request("GET", "activity?principal="+url.QueryEscape(agent.ID), nil)
	add("activity_retains_90_days", err == nil && pruneErr == nil && resp.StatusCode == 200 && bytes.Contains(pruned, []byte(`"activity":[]`)))
	code, _ := world.Request(ctx, principal, "/web/v1/activity", nil, &out)
	add("activity_owner_only", code == 401)
}
