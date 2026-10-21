package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"strings"
	"time"
)

func runAgentReplicaBoundary(ctx context.Context, w *world.World, chatID string, command func() contracts.Command, add func(string, bool)) {
	replica := w.Replicas[0]
	replicaAuthority, replicaErr := trust.Open(replica.Home)
	if replicaErr == nil {
		replicaAgent, replicaToken, createErr := replicaAuthority.CreateAgent("Bridge Test replica", trust.AgentApprove, []contracts.Grant{"messages:write"})
		replicaRead := w.Await(5*time.Second, func() bool {
			status, body, _ := agentRequest(ctx, replica.URL, replicaToken, "POST", "/v1/op/messages.chats", map[string]any{})
			return status == 200 && bytes.Contains(body, []byte(chatID))
		})
		add("replica_agent_local_read", createErr == nil && replicaRead == nil)
		replicaCommand := command()
		replicaStatus, _, _ := agentRequest(ctx, replica.URL, replicaToken, "POST", "/v1/op/messages.send", replicaCommand)
		effects, _ := w.Effects()
		add("replica_agent_write_refused_before_forwarding", replicaStatus == 422 && effects[replicaCommand.ID] == 0)
		ownerToken, _, ownerErr := replicaAuthority.CreateDeviceSession("trusted")
		replicaOwner := &webClient{ctx: ctx, origin: strings.Replace(replica.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: ownerToken}}
		response, activity, activityErr := replicaOwner.request("GET", "activity?principal="+url.QueryEscape(replicaAgent.ID), nil)
		add("replica_refusal_logged", ownerErr == nil && activityErr == nil && response.StatusCode == 200 && bytes.Contains(activity, []byte(replicaCommand.ID)))
	} else {
		add("replica_agent_authority", false)
	}
}

func checkAgentLevelActivity(c *webClient, expected map[string]contracts.CommandState, add func(string, bool)) {
	response, raw, err := c.request("GET", "activity", nil)
	if err != nil || response.StatusCode != 200 {
		add("level_activity_available", false)
		return
	}
	var log struct {
		Rows []struct {
			ID    string                 `json:"id"`
			State contracts.CommandState `json:"state"`
			Count int                    `json:"count"`
			Hash  string                 `json:"argument_hash"`
		} `json:"activity"`
	}
	err = json.Unmarshal(raw, &log)
	found := 0
	for _, row := range log.Rows {
		id := row.ID[strings.LastIndex(row.ID, ":")+1:]
		if wanted, ok := expected[id]; ok && wanted == row.State && row.Count == 1 && row.Hash != "" {
			found++
		}
	}
	add("each_level_command_logged_individually", err == nil && found == len(expected))
	add("level_activity_text_redacted", !bytes.Contains(raw, []byte("owner-approved agent message")))
}

func checkAgentPendingMCP(ctx context.Context, w *world.World, token string, pending contracts.Command, add func(string, bool)) {
	endpoint := w.Gateway
	endpoint.Token = token
	client := scripted.Client{Endpoint: endpoint, Protocol: "mcp"}
	var record contracts.CommandRecord
	err := client.Call(ctx, pending.Op, pending, &record)
	add("mcp_wait_reports_awaiting_approval", err == nil && record.State == contracts.CommandAwaitingApproval && record.Disposition == contracts.DispositionAwaitingApproval)
	err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: pending.ID}, &record)
	add("mcp_status_reports_awaiting_approval", err == nil && record.State == contracts.CommandAwaitingApproval)
}
