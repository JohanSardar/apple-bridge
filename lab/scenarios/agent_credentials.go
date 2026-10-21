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
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runAgentCredentials(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	path := filepath.Join(w.Gateway.Home, "credentials.json")
	before, _ := os.Stat(path)
	var output map[string]any
	status, err := world.Request(ctx, w.Gateway, "/v1/op/messages.chats", map[string]any{}, &output)
	after, _ := os.Stat(path)
	add("legacy_token_auth_stays_read_only", err == nil && status == 200 && before != nil && after != nil && before.ModTime() == after.ModTime())
	samples := []float64{}
	for _, level := range []string{"read", "", "full"} {
		args := []string{"agent", "create", "--home", w.Gateway.Home, "--name", "Bridge Test agent", "--scope", "messages:write"}
		if level != "" {
			args = append(args, "--level", level)
		}
		raw, err := exec.CommandContext(ctx, "bin/bridge", args...).Output()
		var result struct {
			Agent trust.Agent `json:"agent"`
			Token string      `json:"token"`
		}
		json.Unmarshal(raw, &result)
		expected := trust.AgentLevel(level)
		if expected == "" {
			expected = trust.AgentApprove
		}
		add("create_"+string(expected), err == nil && result.Agent.ID != "" && result.Token != "" && result.Agent.Level == expected)
		if result.Token == "" {
			return
		}
		principal := w.Gateway
		principal.Token = result.Token
		for count := 0; count < 3; count++ {
			start := time.Now()
			status, err = world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &output)
			samples = append(samples, float64(time.Since(start))/float64(time.Millisecond))
			add("scoped_read", err == nil && status == 200)
		}
		stored, _ := os.ReadFile(path)
		add("token_stored_hashed", !bytes.Contains(stored, []byte(result.Token)))
		if level == "read" {
			add("read_scope_normalized", len(result.Agent.Scopes) == 1 && result.Agent.Scopes[0] == "messages:read")
		}
		if expected != trust.AgentFull {
			id := contracts.NewCommandID()
			status, _ = world.Request(ctx, principal, "/v1/op/messages.send", map[string]any{"command_id": id, "args": map[string]any{"chat_id": "BridgeTest", "text": "[ab-test credentials] approval required"}}, &output)
			effects, effectsErr := w.Effects()
			add("no_unapproved_effect", effectsErr == nil && effects[id] == 0 && (status == 401 || status == 200 && output["state"] == "awaiting_approval"))
		}
		listed, err := exec.CommandContext(ctx, "bin/bridge", "agent", "list", "--home", w.Gateway.Home).Output()
		add("list_is_redacted", err == nil && bytes.Contains(listed, []byte(result.Agent.ID)) && !bytes.Contains(listed, []byte(result.Token)) && !bytes.Contains(listed, []byte("token_hash")) && bytes.Contains(listed, []byte("last_used")))
		_, err = exec.CommandContext(ctx, "bin/bridge", "agent", "revoke", result.Agent.ID, "--home", w.Gateway.Home).Output()
		status, _ = world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &output)
		add("revoke_stops_access", err == nil && status == 401)
		listed, err = exec.CommandContext(ctx, "bin/bridge", "agent", "list", "--home", w.Gateway.Home).Output()
		add("revoke_time_retained", err == nil && bytes.Contains(listed, []byte("revoked_at")))
	}
	for _, args := range [][]string{{"--level", "invalid", "--scope", "messages:read"}, {"--scope", "admin"}} {
		_, err := exec.CommandContext(ctx, "bin/bridge", append([]string{"agent", "create", "--home", w.Gateway.Home, "--name", "Bridge Test invalid"}, args...)...).Output()
		add("invalid_agent_refused", err != nil)
	}
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("owner_authority", false)
		return
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		add("owner_session", false)
		return
	}
	owner := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
	resp, created, err := owner.request("POST", "agents", map[string]any{"name": "Bridge Test web agent", "scopes": []string{"messages:read"}})
	add("owner_creates_agent", err == nil && resp.StatusCode == 201)
	if err == nil && resp.StatusCode == 201 {
		var creation map[string]any
		json.Unmarshal(created, &creation)
		agent, _ := creation["agent"].(map[string]any)
		id, _ := agent["id"].(string)
		secret, _ := creation["token"].(string)
		resp, listed, err := owner.request("GET", "agents", nil)
		encoded := listed
		add("owner_lists_without_secret", err == nil && resp.StatusCode == 200 && id != "" && secret != "" && bytes.Contains(encoded, []byte(id)) && !bytes.Contains(encoded, []byte(secret)))
		principal := w.Gateway
		principal.Token = secret
		status, _ := world.Request(ctx, principal, "/web/v1/agents", nil, &output)
		add("agent_cannot_manage_owner_routes", status == 401)
		resp, _, err = owner.request("DELETE", "agents/"+id, nil)
		status, _ = world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &output)
		add("owner_revokes_agent", err == nil && resp.StatusCode == 200 && status == 401)
	}
	e.Latency["agent_read"] = stats(samples, 0)
}
