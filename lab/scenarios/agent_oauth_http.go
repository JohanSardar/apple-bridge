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
	"strings"
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
}

func cloneValues(v url.Values) url.Values {
	out := url.Values{}
	for k, values := range v {
		out[k] = append([]string{}, values...)
	}
	return out
}
