package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type connectorApprovalFixture struct {
	ID         string    `json:"id"`
	Code       string    `json:"code"`
	ClientName string    `json:"client_name"`
	Scopes     []string  `json:"scopes"`
	ExpiresAt  time.Time `json:"expires_at"`
}

func runConnectorApproval(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: contracts.NewID("connector-approval"), NetworkPath: "isolated gateway HTTP, owner-only local socket, OAuth and MCP; wall-clock expiry"}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("isolated_gateway", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	_, err = w.GatewayCommand("agent", "list")
	add("authority", err == nil)
	_, err = w.GatewayCommand("connector", "pending")
	add("owner_local_socket", err == nil)
	status, raw, _ := agentRequest(ctx, w.Gateway.URL, "", "POST", "/oauth/register", oauthClientFixture{Name: "[ab-test] " + s.ID, RedirectURIs: []string{"https://connector.example/callback"}, GrantTypes: []string{"authorization_code", "refresh_token"}, TokenEndpointAuthMethod: "none"})
	var client oauthClientFixture
	json.Unmarshal(raw, &client)
	add("registered_client", status == 201 && client.ID != "")
	if client.ID == "" {
		return e
	}
	now := time.Now()
	verifier := strings.Repeat("a", 43)
	challenge := sha256.Sum256([]byte(verifier))
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {"https://connector.example/callback"}, "scope": {"messages:read"}, "resource": {w.Gateway.URL + "/mcp"}, "state": {"[ab-test]"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(challenge[:])}, "code_challenge_method": {"S256"}}
	request := func(method, path string) *httptest.ResponseRecorder {
		recorder := httptest.NewRecorder()
		networkRequest, _ := http.NewRequestWithContext(ctx, method, w.Gateway.URL+path, strings.NewReader("{}"))
		networkRequest.Header.Set("Origin", w.Gateway.URL)
		response, err := http.DefaultClient.Do(networkRequest)
		if err != nil {
			recorder.WriteHeader(503)
			return recorder
		}
		defer response.Body.Close()
		recorder.WriteHeader(response.StatusCode)
		io.Copy(recorder, response.Body)
		return recorder
	}
	add("authorize_without_passkey", request("GET", "/oauth/authorize?"+q.Encode()).Code == 200)
	begin := func() connectorApprovalFixture {
		response := request("POST", "/oauth/pending?"+q.Encode())
		var pending connectorApprovalFixture
		json.Unmarshal(response.Body.Bytes(), &pending)
		add("pending_matching_code_and_requested_data", response.Code == 201 && len(pending.Code) == 4 && pending.ClientName == client.Name && len(pending.Scopes) == 1 && pending.Scopes[0] == "messages:read" && pending.ExpiresAt.Sub(now) >= 5*time.Minute && pending.ExpiresAt.Sub(now) < 5*time.Minute+10*time.Second)
		return pending
	}
	decide := func(p connectorApprovalFixture, code, action string) error {
		_, err := w.GatewayCommand("connector", action, "--request", p.ID, "--code", code)
		return err
	}
	poll := func(p connectorApprovalFixture) *httptest.ResponseRecorder {
		return request("GET", "/oauth/pending/"+p.ID)
	}
	first, other := begin(), begin()
	wrong := "0000"
	if wrong == first.Code {
		wrong = "0001"
	}
	add("wrong_code_refused", decide(first, wrong, "allow") != nil && !bytes.Contains(poll(first).Body.Bytes(), []byte("?code=")))
	add("another_requests_code_refused", decide(first, other.Code, "allow") != nil)
	add("mac_allow", decide(first, first.Code, "allow") == nil)
	pendingList, listErr := w.GatewayCommand("connector", "pending")
	var remaining []connectorApprovalFixture
	json.Unmarshal(pendingList, &remaining)
	add("one_request_never_approves_another", listErr == nil && len(remaining) == 1 && remaining[0].ID == other.ID && !bytes.Contains(poll(other).Body.Bytes(), []byte("code=")))
	add("decision_single_use", decide(first, first.Code, "allow") != nil)
	response := poll(first)
	var result struct {
		Redirect string `json:"redirect"`
	}
	json.Unmarshal(response.Body.Bytes(), &result)
	dest, _ := url.Parse(result.Redirect)
	add("approval_redirect_with_state_and_code", response.Code == 200 && dest != nil && dest.Query().Get("code") != "" && dest.Query().Get("state") == "[ab-test]")
	add("poll_single_use", poll(first).Code == 410)
	form := url.Values{"grant_type": {"authorization_code"}, "client_id": {client.ID}, "code": {dest.Query().Get("code")}, "redirect_uri": {q.Get("redirect_uri")}, "code_verifier": {verifier}, "resource": {q.Get("resource")}}
	status, raw, _ = agentRequest(ctx, w.Gateway.URL, "", "POST", "/oauth/token", form)
	var tokens oauthTokensFixture
	json.Unmarshal(raw, &tokens)
	add("oauth_token", status == 200 && tokens.AccessToken != "")
	status, raw, _ = agentRequest(ctx, w.Gateway.URL, tokens.AccessToken, "POST", "/mcp", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": map[string]any{"name": "records_query", "arguments": map[string]any{"collection": "messages.chat", "limit": 1}}})
	add("token_calls_records_query", status == 200 && bytes.Contains(raw, []byte(`"result"`)) && !bytes.Contains(raw, []byte(`"isError":true`)) && !bytes.Contains(raw, []byte(`"error":`)))
	agentRaw, agentErr := w.GatewayCommand("agent", "list")
	var agents []struct {
		Name   string   `json:"name"`
		Scopes []string `json:"scopes"`
	}
	json.Unmarshal(agentRaw, &agents)
	add("agent_listed_with_requested_scopes", agentErr == nil && len(agents) == 1 && agents[0].Name == client.Name && len(agents[0].Scopes) == 1 && agents[0].Scopes[0] == "messages:read")
	add("mac_deny", decide(other, other.Code, "deny") == nil)
	json.Unmarshal(poll(other).Body.Bytes(), &result)
	denied, _ := url.Parse(result.Redirect)
	add("deny_access_denied", denied != nil && denied.Query().Get("error") == "access_denied" && denied.Query().Get("code") == "")
	now = time.Now()
	expired := begin()
	select {
	case <-ctx.Done():
		return e
	case <-time.After(time.Until(expired.ExpiresAt) + time.Second):
	}
	now = time.Now()
	add("expired_request_refused", decide(expired, expired.Code, "allow") != nil && poll(expired).Code == 410)
	exhausted := begin()
	for i := 0; i < 5; i++ {
		add("wrong_code_attempt_refused", decide(exhausted, "wrong", "allow") != nil)
	}
	add("code_attempts_rate_limited", decide(exhausted, exhausted.Code, "allow") != nil)
	limited := false
	for i := 0; i < 12; i++ {
		limited = request("POST", "/oauth/pending?"+q.Encode()).Code == 429 || limited
	}
	add("creation_rate_limited", limited)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
