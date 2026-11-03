package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"time"
)

type oauthClientFixture struct {
	ID                      string   `json:"client_id,omitempty"`
	Name                    string   `json:"client_name"`
	RedirectURIs            []string `json:"redirect_uris"`
	TokenEndpointAuthMethod string   `json:"token_endpoint_auth_method"`
	GrantTypes              []string `json:"grant_types"`
	ResponseTypes           []string `json:"response_types,omitempty"`
}

type oauthTokensFixture struct {
	AccessToken  string `json:"access_token"`
	TokenType    string `json:"token_type"`
	ExpiresIn    int    `json:"expires_in"`
	RefreshToken string `json:"refresh_token"`
	Scope        string `json:"scope"`
}

type oauthAccessFixture struct {
	Hash      string    `json:"hash"`
	Principal string    `json:"principal"`
	Resource  string    `json:"resource"`
	ExpiresAt time.Time `json:"expires_at"`
}

type oauthRefreshFixture struct {
	Hash      string    `json:"hash"`
	ClientID  string    `json:"client_id"`
	Principal string    `json:"principal"`
	Resource  string    `json:"resource"`
	ExpiresAt time.Time `json:"expires_at"`
	Used      bool      `json:"used"`
}

func fixtureTokenHash(token []byte) string { return fmt.Sprintf("%x", sha256.Sum256(token)) }

type binaryOAuthFixture struct {
	ctx    context.Context
	origin string
	owner  *webClient
}

func newBinaryOAuthFixture(ctx context.Context, w *world.World, origin string) (*binaryOAuthFixture, error) {
	owner, err := binaryOwnerSession(ctx, w, "trusted")
	if err != nil {
		return nil, err
	}
	owner.origin = origin
	return &binaryOAuthFixture{ctx: ctx, origin: origin, owner: owner}, nil
}

func (f *binaryOAuthFixture) register(client oauthClientFixture) (oauthClientFixture, error) {
	client.TokenEndpointAuthMethod = "none"
	status, raw, _ := agentRequest(f.ctx, f.origin, "", "POST", "/oauth/register", client)
	var result oauthClientFixture
	if err := json.Unmarshal(raw, &result); err != nil {
		return result, err
	}
	if status != 201 || result.ID == "" {
		return result, fmt.Errorf("binary OAuth registration status %d", status)
	}
	return result, nil
}

func (f *binaryOAuthFixture) issue(client, redirect, challenge, resource string) (string, error) {
	query := url.Values{"client_id": {client}, "response_type": {"code"}, "redirect_uri": {redirect}, "scope": {"messages:read"}, "resource": {resource}, "state": {"bridge-test-grant"}, "code_challenge": {challenge}, "code_challenge_method": {"S256"}}
	raw, err := json.Marshal(map[string]any{"decision": "approve", "level": "read", "scopes": []string{"messages:read"}})
	if err != nil {
		return "", err
	}
	request, err := http.NewRequestWithContext(f.ctx, "POST", f.origin+"/oauth/authorize?"+query.Encode(), bytes.NewReader(raw))
	if err != nil {
		return "", err
	}
	request.Header.Set("Origin", f.origin)
	request.Header.Set("Content-Type", "application/json")
	request.AddCookie(f.owner.cookie)
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		return "", err
	}
	defer response.Body.Close()
	var result struct {
		Redirect string `json:"redirect"`
	}
	if err = json.NewDecoder(response.Body).Decode(&result); err != nil {
		return "", err
	}
	if response.StatusCode != 200 {
		return "", fmt.Errorf("binary OAuth consent status %d", response.StatusCode)
	}
	link, err := url.Parse(result.Redirect)
	if err != nil {
		return "", err
	}
	code := link.Query().Get("code")
	if code == "" {
		return "", fmt.Errorf("binary OAuth code missing")
	}
	return code, nil
}

func (f *binaryOAuthFixture) exchange(grant, client, code, redirect, verifier, resource, refresh, scope string) (oauthTokensFixture, error) {
	form := url.Values{"grant_type": {grant}, "client_id": {client}, "code": {code}, "redirect_uri": {redirect}, "code_verifier": {verifier}, "resource": {resource}, "refresh_token": {refresh}, "scope": {scope}}
	status, raw, _ := agentRequest(f.ctx, f.origin, "", "POST", "/oauth/token", form)
	var result oauthTokensFixture
	if err := json.Unmarshal(raw, &result); err != nil {
		return result, err
	}
	if status != 200 {
		return result, fmt.Errorf("binary OAuth token status %d", status)
	}
	return result, nil
}

func binaryOAuthPrincipal(ctx context.Context, w *world.World, origin, token string) (string, error) {
	status, _, _ := agentRequest(ctx, origin, token, "POST", "/v1/op/messages.chats", map[string]any{})
	if status != 200 {
		return "", fmt.Errorf("binary OAuth authentication status %d", status)
	}
	raw, err := os.ReadFile(filepath.Join(w.Gateway.Home, "credentials.json"))
	if err != nil {
		return "", err
	}
	var credentials struct {
		Access []oauthAccessFixture `json:"oauth_access"`
	}
	if err = json.Unmarshal(raw, &credentials); err != nil {
		return "", err
	}
	for _, access := range credentials.Access {
		if access.Hash == fixtureTokenHash([]byte(token)) && access.Principal != "" {
			return access.Principal, nil
		}
	}
	return "", fmt.Errorf("binary OAuth principal missing")
}
