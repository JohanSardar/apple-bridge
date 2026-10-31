package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http/httptest"
	"net/url"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runConnectorApproval(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: contracts.NewID("connector-approval"), NetworkPath: "isolated authorization HTTP handler, owner-only Mac socket, OAuth and MCP gateway"}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("isolated_gateway", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	authority, err := trust.Open(w.Gateway.Home)
	add("authority", err == nil)
	if err != nil {
		return e
	}
	approvals := bridgeweb.NewConnectorApprovals(authority)
	now := time.Now()
	approvals.Now = func() time.Time { return now }
	origin, _ := url.Parse(w.Gateway.URL)
	handler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, ConnectorApprovals: approvals, Address: origin.Host})
	stop, err := bridgeweb.ServeConnectorApprovals(w.Dir, approvals)
	add("owner_local_socket", err == nil)
	if err != nil {
		return e
	}
	defer stop()
	client, err := authority.RegisterOAuth(trust.OAuthClient{Name: "[ab-test] " + s.ID, RedirectURIs: []string{"https://connector.example/callback"}, GrantTypes: []string{"authorization_code", "refresh_token"}})
	add("registered_client", err == nil)
	verifier := strings.Repeat("a", 43)
	challenge := sha256.Sum256([]byte(verifier))
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {"https://connector.example/callback"}, "scope": {"messages:read"}, "resource": {w.Gateway.URL + "/mcp"}, "state": {"[ab-test]"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(challenge[:])}, "code_challenge_method": {"S256"}}
	request := func(method, path string) *httptest.ResponseRecorder {
		req := httptest.NewRequest(method, w.Gateway.URL+path, strings.NewReader("{}"))
		req.RemoteAddr = "127.0.0.1:12345"
		req.Header.Set("Origin", w.Gateway.URL)
		recorder := httptest.NewRecorder()
		handler.ServeHTTP(recorder, req)
		return recorder
	}
	add("authorize_without_passkey", request("GET", "/oauth/authorize?"+q.Encode()).Code == 200)
	begin := func() bridgeweb.ConnectorApproval {
		response := request("POST", "/oauth/pending?"+q.Encode())
		var pending bridgeweb.ConnectorApproval
		json.Unmarshal(response.Body.Bytes(), &pending)
		add("pending_matching_code_and_requested_data", response.Code == 201 && len(pending.Code) == 4 && pending.ClientName == client.Name && len(pending.Scopes) == 1 && pending.Scopes[0] == "messages:read" && pending.ExpiresAt.Sub(now) == 5*time.Minute)
		return pending
	}
	decide := func(p bridgeweb.ConnectorApproval, code, action string) error {
		return bridgeweb.ConnectorCommand(ctx, w.Dir, action, p.ID, code, io.Discard)
	}
	poll := func(p bridgeweb.ConnectorApproval) *httptest.ResponseRecorder {
		return request("GET", "/oauth/pending/"+p.ID)
	}
	first, other := begin(), begin()
	wrong := "0000"
	if wrong == first.Code {
		wrong = "0001"
	}
	add("wrong_code_refused", decide(first, wrong, "allow") != nil && !bytes.Contains(poll(first).Body.Bytes(), []byte("?code=")))
	add("mac_allow", decide(first, first.Code, "allow") == nil)
	add("one_request_never_approves_another", len(approvals.Pending()) == 1 && approvals.Pending()[0].ID == other.ID && !bytes.Contains(poll(other).Body.Bytes(), []byte("code=")))
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
	status, raw, _ := agentRequest(ctx, w.Gateway.URL, "", "POST", "/oauth/token", form)
	var tokens trust.OAuthTokens
	json.Unmarshal(raw, &tokens)
	add("oauth_token", status == 200 && tokens.AccessToken != "")
	status, raw, _ = agentRequest(ctx, w.Gateway.URL, tokens.AccessToken, "POST", "/mcp", map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": map[string]any{"name": "records_query", "arguments": map[string]any{"collection": "messages.chat", "limit": 1}}})
	add("token_calls_records_query", status == 200 && bytes.Contains(raw, []byte(`"result"`)) && !bytes.Contains(raw, []byte(`"isError":true`)) && !bytes.Contains(raw, []byte(`"error":`)))
	agents, _ := authority.Agents()
	add("agent_listed_with_requested_scopes", len(agents) == 1 && agents[0].Name == client.Name && len(agents[0].Scopes) == 1 && agents[0].Scopes[0] == "messages:read")
	add("mac_deny", decide(other, other.Code, "deny") == nil)
	json.Unmarshal(poll(other).Body.Bytes(), &result)
	denied, _ := url.Parse(result.Redirect)
	add("deny_access_denied", denied != nil && denied.Query().Get("error") == "access_denied" && denied.Query().Get("code") == "")
	expired := begin()
	now = now.Add(5 * time.Minute)
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
