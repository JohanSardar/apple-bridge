package scenarios

import (
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
)

type realPhoneConnector struct {
	authority *trust.Authority
	approvals *bridgeweb.ConnectorApprovals
	handler   http.Handler
	client    trust.OAuthClient
	pending   bridgeweb.ConnectorApproval
	verifier  string
}

func newRealPhoneConnector(authority *trust.Authority) (*realPhoneConnector, error) {
	client, err := authority.RegisterOAuth(trust.OAuthClient{Name: "[ab-test] iPhone connection", RedirectURIs: []string{"https://connector.example/callback"}, GrantTypes: []string{"authorization_code", "refresh_token"}})
	if err != nil {
		return nil, err
	}
	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		return nil, err
	}
	verifier := base64.RawURLEncoding.EncodeToString(secret)
	challenge := sha256.Sum256([]byte(verifier))
	approvals := bridgeweb.NewConnectorApprovals(authority)
	handler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, ConnectorApprovals: approvals, Address: "127.0.0.1:7780"})
	query := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {client.RedirectURIs[0]}, "code_challenge": {base64.RawURLEncoding.EncodeToString(challenge[:])}, "code_challenge_method": {"S256"}, "resource": {"http://127.0.0.1:7780/mcp"}, "scope": {"messages:read"}, "state": {"ab-test"}}
	request := httptest.NewRequest("POST", "http://127.0.0.1:7780/oauth/pending?"+query.Encode(), strings.NewReader("{}"))
	request.Header.Set("Origin", "http://127.0.0.1:7780")
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, request)
	var pending bridgeweb.ConnectorApproval
	if response.Code != http.StatusCreated || json.Unmarshal(response.Body.Bytes(), &pending) != nil || !strings.HasPrefix(pending.ID, "connector:") {
		return nil, contracts.ErrUnavailable
	}
	return &realPhoneConnector{authority: authority, approvals: approvals, handler: handler, client: client, pending: pending, verifier: verifier}, nil
}

func (f *realPhoneConnector) pendingPhone() []iphone.ConnectorApproval {
	pending := f.approvals.Pending()
	result := make([]iphone.ConnectorApproval, 0, len(pending))
	for _, p := range pending {
		result = append(result, iphone.ConnectorApproval{ID: p.ID, Code: p.Code, Name: p.ClientName, Scopes: p.Scopes, Expires: p.ExpiresAt})
	}
	return result
}

func (f *realPhoneConnector) qualified() bool {
	response := httptest.NewRecorder()
	f.handler.ServeHTTP(response, httptest.NewRequest("GET", "http://127.0.0.1:7780/oauth/pending/"+url.PathEscape(f.pending.ID), nil))
	var result struct {
		Redirect string `json:"redirect"`
	}
	if response.Code != http.StatusOK || json.Unmarshal(response.Body.Bytes(), &result) != nil {
		return false
	}
	redirect, err := url.Parse(result.Redirect)
	if err != nil || redirect.Query().Get("state") != "ab-test" || redirect.Query().Get("code") == "" {
		return false
	}
	token, err := f.authority.ExchangeOAuth("authorization_code", f.client.ID, redirect.Query().Get("code"), f.client.RedirectURIs[0], f.verifier, "http://127.0.0.1:7780/mcp", "", "")
	if err != nil {
		return false
	}
	principal, err := f.authority.AuthenticateToken(token.AccessToken)
	return err == nil && len(principal.Grants) == 1 && principal.Grants[0] == "messages:read"
}
