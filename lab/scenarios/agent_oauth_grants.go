package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func runOAuthGrants(ctx context.Context, w *world.World, authority *trust.Authority, add func(string, bool)) {
	redirect := "https://connector.example/callback"
	client, err := authority.RegisterOAuth(trust.OAuthClient{Name: "Bridge Test connector", RedirectURIs: []string{redirect}, GrantTypes: []string{"authorization_code", "refresh_token"}})
	add("oauth_grant_client_registration", err == nil && client.ID != "")
	if err != nil {
		return
	}
	verifier := strings.Repeat("a", 43)
	challenge := sha256.Sum256([]byte(verifier))
	resource := w.Gateway.URL + "/mcp"
	issue := func() (trust.OAuthTokens, string, error) {
		agent, _, err := authority.CreateAgent("Bridge Test OAuth grant", trust.AgentRead, []contracts.Grant{"messages:read"})
		if err != nil {
			return trust.OAuthTokens{}, "", err
		}
		code, err := authority.IssueOAuthCode(trust.OAuthCode{ClientID: client.ID, RedirectURI: redirect, Challenge: base64.RawURLEncoding.EncodeToString(challenge[:]), Resource: resource, Principal: agent.ID})
		if err != nil {
			return trust.OAuthTokens{}, "", err
		}
		_, wrong := authority.ExchangeOAuth("authorization_code", client.ID, code, redirect, strings.Repeat("b", 43), resource, "", "")
		add("oauth_grant_wrong_pkce_rejected", wrong != nil)
		token, err := authority.ExchangeOAuth("authorization_code", client.ID, code, redirect, verifier, resource, "", "")
		return token, code, err
	}
	read := func(token string) int {
		endpoint := w.Gateway
		endpoint.Token = token
		var out map[string]any
		code, _ := world.Request(ctx, endpoint, "/v1/op/messages.chats", map[string]any{}, &out)
		return code
	}
	tokens, _, err := issue()
	add("oauth_grant_exchanged_and_readable", err == nil && tokens.ExpiresIn == 3600 && read(tokens.AccessToken) == 200)
	if err != nil {
		return
	}
	stored, err := os.ReadFile(filepath.Join(w.Gateway.Home, "credentials.json"))
	var data struct {
		Access  []trust.OAuthAccess  `json:"oauth_access"`
		Refresh []trust.OAuthRefresh `json:"oauth_refresh"`
	}
	json.Unmarshal(stored, &data)
	accessTTL, refreshTTL := false, false
	for _, v := range data.Access {
		if v.Hash == trust.Fingerprint([]byte(tokens.AccessToken)) {
			accessTTL = time.Until(v.ExpiresAt) > 59*time.Minute && time.Until(v.ExpiresAt) <= time.Hour
		}
	}
	for _, v := range data.Refresh {
		if v.Hash == trust.Fingerprint([]byte(tokens.RefreshToken)) {
			refreshTTL = time.Until(v.ExpiresAt) > 89*24*time.Hour && time.Until(v.ExpiresAt) <= 90*24*time.Hour
		}
	}
	add("oauth_grant_lifetimes", err == nil && accessTTL && refreshTTL)
	add("oauth_grant_secrets_hashed", !strings.Contains(string(stored), tokens.AccessToken) && !strings.Contains(string(stored), tokens.RefreshToken))
	rotated, err := authority.ExchangeOAuth("refresh_token", client.ID, "", "", "", resource, tokens.RefreshToken, "")
	add("oauth_grant_refresh_rotates", err == nil && rotated.RefreshToken != tokens.RefreshToken && rotated.AccessToken != tokens.AccessToken && read(rotated.AccessToken) == 200)
	_, err = authority.ExchangeOAuth("refresh_token", client.ID, "", "", "", resource, tokens.RefreshToken, "")
	add("oauth_grant_refresh_reuse_revokes", err != nil && read(rotated.AccessToken) == 401)
	_, err = authority.ExchangeOAuth("refresh_token", client.ID, "", "", "", resource, rotated.RefreshToken, "")
	add("oauth_grant_refresh_family_revoked", err != nil)
	tokens, code, err := issue()
	_, replay := authority.ExchangeOAuth("authorization_code", client.ID, code, redirect, verifier, resource, "", "")
	add("oauth_grant_code_reuse_revokes", err == nil && replay != nil && read(tokens.AccessToken) == 401)
	tokens, _, err = issue()
	principal, authErr := authority.AuthenticateToken(tokens.AccessToken)
	revokeErr := authority.Revoke(principal.ID)
	add("oauth_grant_owner_revocation", err == nil && authErr == nil && revokeErr == nil && read(tokens.AccessToken) == 401)
}
