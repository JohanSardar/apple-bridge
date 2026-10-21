package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

func runOAuthCredentialAccess(ctx context.Context, w *world.World, authority *trust.Authority, add func(string, bool)) {
	agent, token, err := authority.CreateAgent("Bridge Test OAuth credential", trust.AgentRead, []contracts.Grant{"messages:read"})
	if err != nil {
		add("oauth_credential_agent", false)
		return
	}
	err = editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, value := range data["principals"].([]any) {
			principal := value.(map[string]any)
			if principal["id"] == agent.ID {
				principal["token_hash"] = ""
			}
		}
		data["oauth_access"] = []any{map[string]any{"hash": trust.Fingerprint([]byte(token)), "principal": agent.ID, "resource": w.Gateway.URL + "/mcp", "expires_at": time.Now().Add(time.Hour).Format(time.RFC3339Nano)}}
	})
	principal := w.Gateway
	principal.Token = token
	var result map[string]any
	code, requestErr := world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &result)
	add("oauth_opaque_access_authenticates", err == nil && requestErr == nil && code == 200)
	if code != 200 {
		return
	}
	change := func(resource string, expiry time.Time) error {
		return editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
			access := data["oauth_access"].([]any)[0].(map[string]any)
			access["resource"] = resource
			access["expires_at"] = expiry.Format(time.RFC3339Nano)
		})
	}
	err = change("https://attacker.example/mcp", time.Now().Add(time.Hour))
	code, _ = world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &result)
	add("oauth_opaque_resource_bound", err == nil && code == 401)
	err = change(w.Gateway.URL+"/mcp", time.Now().Add(-time.Second))
	code, _ = world.Request(ctx, principal, "/v1/op/messages.chats", map[string]any{}, &result)
	add("oauth_opaque_access_expires", err == nil && code == 401)
}

func editAgentCredentials(home string, edit func(map[string]any)) error {
	lock, err := os.OpenFile(filepath.Join(home, "credentials.lock"), os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX); err != nil {
		return err
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	path := filepath.Join(home, "credentials.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var data map[string]any
	if err = json.Unmarshal(raw, &data); err != nil {
		return err
	}
	edit(data)
	raw, err = json.Marshal(data)
	if err != nil {
		return err
	}
	if err = os.WriteFile(path+".agent-expiry", raw, 0600); err != nil {
		return err
	}
	return os.Rename(path+".agent-expiry", path)
}
