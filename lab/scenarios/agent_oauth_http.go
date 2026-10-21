package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func runAgentOAuth(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	for _, path := range []string{"/mcp", "/v1/op/messages.chats"} {
		status, _, header := agentRequest(ctx, c.origin, "", "POST", path, map[string]any{})
		add("oauth_discovery_challenge_"+path, status == 401 && strings.Contains(header.Get("WWW-Authenticate"), c.origin+"/.well-known/oauth-protected-resource"))
	}
	code, raw, _ := agentRequest(ctx, c.origin, "", "GET", "/.well-known/oauth-protected-resource", nil)
	add("resource_metadata", code == 200 && bytes.Contains(raw, []byte(c.origin+"/mcp")))
	code, raw, _ = agentRequest(ctx, c.origin, "", "GET", "/.well-known/oauth-authorization-server", nil)
	add("authorization_metadata_pkce", code == 200 && bytes.Contains(raw, []byte("S256")))
	redirect := "https://connector.example/callback"
	code, raw, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/register", map[string]any{"client_name": "Bridge Test connector", "redirect_uris": []string{redirect}, "grant_types": []string{"authorization_code", "refresh_token"}, "token_endpoint_auth_method": "none"})
	var client trust.OAuthClient
	json.Unmarshal(raw, &client)
	add("dynamic_registration", code == 201 && client.ID != "")
	if client.ID == "" {
		return
	}
	verifier := strings.Repeat("a", 43)
	hash := sha256.Sum256([]byte(verifier))
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {redirect}, "scope": {"messages:read"}, "resource": {c.origin + "/mcp"}, "state": {"bridge-test"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(hash[:])}, "code_challenge_method": {"S256"}}
	request := func(method string, q url.Values, input any) (int, []byte) {
		var body io.Reader
		if input != nil {
			b, _ := json.Marshal(input)
			body = bytes.NewReader(b)
		}
		req, _ := http.NewRequestWithContext(ctx, method, c.origin+"/oauth/authorize?"+q.Encode(), body)
		req.Header.Set("Origin", c.origin)
		req.Header.Set("Content-Type", "application/json")
		req.AddCookie(c.cookie)
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0, nil
		}
		defer resp.Body.Close()
		raw, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, raw
	}
	code, _, header := agentRequest(ctx, c.origin, "", "GET", "/oauth/authorize?"+q.Encode(), nil)
	add("authorize_requires_device_session", code == 303 && strings.HasPrefix(header.Get("Location"), "/signin?return_to="))
	for _, bad := range []string{"plain", ""} {
		v := cloneValues(q)
		v.Set("code_challenge_method", bad)
		code, _ = request("GET", v, nil)
		add("reject_non_s256_"+bad, code == 400)
	}
	bad := cloneValues(q)
	bad.Set("redirect_uri", "https://attacker.example/callback")
	code, _ = request("GET", bad, nil)
	add("redirect_exact_match", code == 400)
	code, raw = request("POST", q, map[string]any{"decision": "approve", "level": "full", "scopes": []string{"contacts:write"}})
	add("consent_cannot_exceed_requested_scopes", code == 400)
	issue := func() (string, url.Values) {
		code, raw := request("POST", q, map[string]any{"decision": "approve", "level": "read", "scopes": []string{"messages:read"}})
		var value struct {
			Redirect string `json:"redirect"`
		}
		json.Unmarshal(raw, &value)
		u, _ := url.Parse(value.Redirect)
		if u == nil {
			return "", nil
		}
		return fmt.Sprint(code), u.Query()
	}
	result, response := issue()
	add("owner_consent_binds_state_issuer", result == "200" && response.Get("state") == "bridge-test" && response.Get("iss") == c.origin && response.Get("code") != "")
	form := url.Values{"grant_type": {"authorization_code"}, "client_id": {client.ID}, "code": {response.Get("code")}, "redirect_uri": {redirect}, "code_verifier": {verifier}, "resource": {c.origin + "/mcp"}}
	bad = cloneValues(form)
	bad.Set("code_verifier", strings.Repeat("b", 43))
	code, _, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", bad)
	add("pkce_wrong_verifier_rejected", code == 400)
	bad = cloneValues(form)
	bad.Set("resource", "https://attacker.example/mcp")
	code, _, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", bad)
	add("resource_binding", code == 400)
	code, raw, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", form)
	var tokens trust.OAuthTokens
	json.Unmarshal(raw, &tokens)
	add("code_exchange_one_hour_access", code == 200 && tokens.AccessToken != "" && tokens.ExpiresIn == 3600 && tokens.RefreshToken != "")
	mcp := func(token, method string, params any) bool {
		code, raw, _ := agentRequest(ctx, c.origin, token, "POST", "/mcp", map[string]any{"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
		return code == 200 && bytes.Contains(raw, []byte(`"result"`)) && !bytes.Contains(raw, []byte(`"isError":true`))
	}
	add("oauth_mcp_tools_list", mcp(tokens.AccessToken, "tools/list", map[string]any{}))
	add("oauth_mcp_read", mcp(tokens.AccessToken, "tools/call", map[string]any{"name": "messages_chats", "arguments": map[string]any{}}))
	stored, err := os.ReadFile(filepath.Join(w.Gateway.Home, "credentials.json"))
	add("oauth_secrets_stored_hashed", err == nil && !bytes.Contains(stored, []byte(tokens.AccessToken)) && !bytes.Contains(stored, []byte(tokens.RefreshToken)) && !bytes.Contains(stored, []byte(response.Get("code"))))
	var credentials struct {
		Access  []trust.OAuthAccess  `json:"oauth_access"`
		Refresh []trust.OAuthRefresh `json:"oauth_refresh"`
	}
	json.Unmarshal(stored, &credentials)
	lifetimes := len(credentials.Access) > 0 && len(credentials.Refresh) > 0
	if lifetimes {
		lifetimes = time.Until(credentials.Access[0].ExpiresAt) > 59*time.Minute && time.Until(credentials.Access[0].ExpiresAt) <= time.Hour && time.Until(credentials.Refresh[0].ExpiresAt) > 89*24*time.Hour && time.Until(credentials.Refresh[0].ExpiresAt) <= 90*24*time.Hour
	}
	add("oauth_one_hour_and_90_day_expiry", lifetimes)
	expired := editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, v := range data["oauth_access"].([]any) {
			v.(map[string]any)["expires_at"] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
		}
	})
	code, _, _ = agentRequest(ctx, c.origin, tokens.AccessToken, "POST", "/v1/op/messages.chats", map[string]any{})
	add("expired_access_rejected", expired == nil && code == 401)
	refresh := url.Values{"grant_type": {"refresh_token"}, "client_id": {client.ID}, "refresh_token": {tokens.RefreshToken}, "resource": {c.origin + "/mcp"}}
	code, raw, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", refresh)
	var rotated trust.OAuthTokens
	json.Unmarshal(raw, &rotated)
	add("refresh_rotates", code == 200 && rotated.RefreshToken != "" && rotated.RefreshToken != tokens.RefreshToken && rotated.AccessToken != tokens.AccessToken)
	code, _, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", refresh)
	add("refresh_reuse_detected", code == 400)
	code, _, _ = agentRequest(ctx, c.origin, rotated.AccessToken, "POST", "/v1/op/messages.chats", map[string]any{})
	add("reuse_revokes_access_family", code == 401)
	refresh.Set("refresh_token", rotated.RefreshToken)
	code, _, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", refresh)
	add("reuse_revokes_refresh_family", code == 400)
	_, response = issue()
	form.Set("code", response.Get("code"))
	code, raw, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", form)
	json.Unmarshal(raw, &tokens)
	replayedCode := cloneValues(form)
	_, extraResponse := issue()
	extraForm := cloneValues(form)
	extraForm.Set("code", extraResponse.Get("code"))
	extraStatus, extraRaw, _ := agentRequest(ctx, c.origin, "", "POST", "/oauth/token", extraForm)
	var extraTokens trust.OAuthTokens
	json.Unmarshal(extraRaw, &extraTokens)
	err = editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, v := range data["oauth_refresh"].([]any) {
			f := v.(map[string]any)
			if f["hash"] == trust.Fingerprint([]byte(extraTokens.RefreshToken)) {
				f["expires_at"] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
			}
		}
	})
	expiredRefresh := url.Values{"grant_type": {"refresh_token"}, "client_id": {client.ID}, "refresh_token": {extraTokens.RefreshToken}, "resource": {c.origin + "/mcp"}}
	code, _, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", expiredRefresh)
	add("expired_refresh_rejected", extraStatus == 200 && err == nil && code == 400)
	code, _, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", replayedCode)
	add("authorization_code_single_use", code == 400)
	code, _, _ = agentRequest(ctx, c.origin, tokens.AccessToken, "POST", "/v1/op/messages.chats", map[string]any{})
	add("code_reuse_revokes_family", code == 401)
	_, response = issue()
	form.Set("code", response.Get("code"))
	code, raw, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", form)
	json.Unmarshal(raw, &tokens)
	a, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("owner_revoke_authority", false)
		return
	}
	p, err := a.AuthenticateToken(tokens.AccessToken)
	if err != nil {
		add("owner_revoke_token", false)
		return
	}
	resp, _, err := c.request("DELETE", "agents/"+p.ID, nil)
	code, _, _ = agentRequest(ctx, c.origin, tokens.AccessToken, "POST", "/v1/op/messages.chats", map[string]any{})
	add("owner_revoke_oauth", err == nil && resp.StatusCode == 200 && code == 401)
}

func cloneValues(v url.Values) url.Values {
	out := url.Values{}
	for k, values := range v {
		out[k] = append([]string{}, values...)
	}
	return out
}
