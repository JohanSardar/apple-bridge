package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"path/filepath"
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
		args := []string{"agent", "create", "--name", "Bridge Test agent", "--scope", "messages:write"}
		if level != "" {
			args = append(args, "--level", level)
		}
		raw, err := w.GatewayCommand(args...)
		var result struct {
			Agent binaryAgentIdentity `json:"agent"`
			Token string              `json:"token"`
		}
		json.Unmarshal(raw, &result)
		expected := level
		if expected == "" {
			expected = "approve"
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
		if expected != "full" {
			id := contracts.NewCommandID()
			status, _ = world.Request(ctx, principal, "/v1/op/messages.send", map[string]any{"command_id": id, "args": map[string]any{"chat_id": "BridgeTest", "text": "[ab-test credentials] approval required"}}, &output)
			effects, effectsErr := w.Effects()
			add("no_unapproved_effect", effectsErr == nil && effects[id] == 0 && (status == 401 || status == 200 && output["state"] == "awaiting_approval"))
		}
		listed, err := w.GatewayCommand("agent", "list")
		add("list_is_redacted", err == nil && bytes.Contains(listed, []byte(result.Agent.ID)) && !bytes.Contains(listed, []byte(result.Token)) && !bytes.Contains(listed, []byte("token_hash")) && bytes.Contains(listed, []byte("last_used")))
		_, err = w.GatewayCommand("agent", "revoke", result.Agent.ID)
		status, _ = world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &output)
		add("revoke_stops_access", err == nil && status == 401)
		listed, err = w.GatewayCommand("agent", "list")
		add("revoke_time_retained", err == nil && bytes.Contains(listed, []byte("revoked_at")))
	}
	for _, args := range [][]string{{"--level", "invalid", "--scope", "messages:read"}, {"--scope", "admin"}} {
		_, err := w.GatewayCommand(append([]string{"agent", "create", "--name", "Bridge Test invalid"}, args...)...)
		add("invalid_agent_refused", err != nil)
	}
	_, err = w.GatewayCommand("agent", "list")
	if err != nil {
		add("owner_authority", false)
		return
	}
	e.Latency["agent_read"] = stats(samples, 0)
	runOAuthCredentialAccess(ctx, w, add)
}
