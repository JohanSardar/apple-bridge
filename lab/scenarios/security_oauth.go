package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func securityOAuth(ctx context.Context, w *world.World, a *trust.Authority, origin string, add func(string, bool)) {
	client, err := a.RegisterOAuth(trust.OAuthClient{Name: "[ab-test] security", RedirectURIs: []string{"https://connector.example/callback"}})
	add("security_oauth_client", err == nil)
	if err != nil {
		return
	}
	verifier := strings.Repeat("a", 43)
	challenge := sha256.Sum256([]byte(verifier))
	resource := origin + "/mcp"
	request := func(token, collection string) int {
		status, _, _ := agentRequest(ctx, origin, token, "POST", "/v1/op/records.query", map[string]any{"collection": collection, "limit": 1})
		return status
	}
	issue := func() (trust.Agent, trust.OAuthCode, string) {
		agent, _, err := a.CreateAgent("[ab-test] security", trust.AgentRead, []contracts.Grant{"messages:read"})
		add("security_agent_created", err == nil)
		code := trust.OAuthCode{ClientID: client.ID, RedirectURI: client.RedirectURIs[0], Resource: resource, Principal: agent.ID, Challenge: base64.RawURLEncoding.EncodeToString(challenge[:])}
		raw, err := a.IssueOAuthCode(code)
		add("security_code_created", err == nil)
		return agent, code, raw
	}
	exchange := func(raw, redirect, proof, target, id string) (int, trust.OAuthTokens) {
		form := url.Values{"grant_type": {"authorization_code"}, "client_id": {id}, "code": {raw}, "redirect_uri": {redirect}, "code_verifier": {proof}, "resource": {target}}
		status, body, _ := agentRequest(ctx, origin, "", "POST", "/oauth/token", form)
		var tokens trust.OAuthTokens
		json.Unmarshal(body, &tokens)
		return status, tokens
	}
	agent, code, raw := issue()
	for _, bad := range []struct{ redirect, proof, target, id string }{
		{"https://attacker.example/callback", verifier, resource, client.ID},
		{code.RedirectURI, strings.Repeat("b", 43), resource, client.ID},
		{code.RedirectURI, "", resource, client.ID},
		{code.RedirectURI, verifier, "https://attacker.example/mcp", client.ID},
		{code.RedirectURI, verifier, resource, "wrong-client"},
	} {
		status, _ := exchange(raw, bad.redirect, bad.proof, bad.target, bad.id)
		add("mismatched_oauth_binding_refused", status == 400)
	}
	status, tokens := exchange(raw, code.RedirectURI, verifier, resource, client.ID)
	add("valid_oauth_access", status == 200 && request(tokens.AccessToken, "messages.chat") == 200)
	if status != 200 {
		return
	}
	wrongScope, _, _ := agentRequest(ctx, origin, tokens.AccessToken, "POST", "/v1/op/iphone.devices", map[string]any{})
	add("wrong_scope_refused", wrongScope == 401 || wrongScope == 403)
	add("garbage_token_refused", request("[ab-test] invalid", "messages.chat") == 401)
	expired := editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, v := range data["oauth_access"].([]any) {
			if v.(map[string]any)["hash"] == trust.Fingerprint([]byte(tokens.AccessToken)) {
				v.(map[string]any)["expires_at"] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
			}
		}
	})
	add("expired_access_refused", expired == nil && request(tokens.AccessToken, "messages.chat") == 401)
	refresh := url.Values{"grant_type": {"refresh_token"}, "client_id": {client.ID}, "refresh_token": {tokens.RefreshToken}, "resource": {resource}}
	status, body, _ := agentRequest(ctx, origin, "", "POST", "/oauth/token", refresh)
	var rotated trust.OAuthTokens
	json.Unmarshal(body, &rotated)
	add("refresh_rotation_works", status == 200 && request(rotated.AccessToken, "messages.chat") == 200)
	status, _, _ = agentRequest(ctx, origin, "", "POST", "/oauth/token", refresh)
	add("replayed_refresh_refused_and_family_revoked", status == 400 && request(rotated.AccessToken, "messages.chat") == 401)
	agent, code, raw = issue()
	status, tokens = exchange(raw, code.RedirectURI, verifier, resource, client.ID)
	replay, _ := exchange(raw, code.RedirectURI, verifier, resource, client.ID)
	add("replayed_code_refused_and_family_revoked", status == 200 && replay == 400 && request(tokens.AccessToken, "messages.chat") == 401)
	agent, code, raw = issue()
	expired = editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, v := range data["oauth_codes"].([]any) {
			if v.(map[string]any)["hash"] == trust.Fingerprint([]byte(raw)) {
				v.(map[string]any)["expires_at"] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
			}
		}
	})
	status, _ = exchange(raw, code.RedirectURI, verifier, resource, client.ID)
	add("expired_code_refused", expired == nil && status == 400)
	agent, code, raw = issue()
	status, tokens = exchange(raw, code.RedirectURI, verifier, resource, client.ID)
	add("connector_works_before_revocation", status == 200 && request(tokens.AccessToken, "messages.chat") == 200)
	start := time.Now()
	err = a.Revoke(agent.ID)
	add("revocation_stops_public_token_within_seconds", err == nil && request(tokens.AccessToken, "messages.chat") == 401 && time.Since(start) < 3*time.Second)
	refresh.Set("refresh_token", tokens.RefreshToken)
	status, _, _ = agentRequest(ctx, origin, "", "POST", "/oauth/token", refresh)
	add("revoked_refresh_refused", status == 400)
	safe := true
	stored, err := os.ReadFile(filepath.Join(w.Gateway.Home, "credentials.json"))
	safe = safe && err == nil
	for _, secret := range []string{raw, tokens.AccessToken, tokens.RefreshToken, rotated.AccessToken, rotated.RefreshToken} {
		safe = safe && secret != "" && !bytes.Contains(stored, []byte(secret))
		log, err := os.ReadFile(filepath.Join(w.Dir, "gateway.log"))
		safe = safe && err == nil && !bytes.Contains(log, []byte(secret))
	}
	add("oauth_codes_and_tokens_absent_from_storage_and_logs", safe)
}
