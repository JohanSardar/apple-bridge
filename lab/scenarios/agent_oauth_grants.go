package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func runOAuthGrants(ctx context.Context, w *world.World, add func(string, bool)) {
	oauth, err := newBinaryOAuthFixture(ctx, w, w.Gateway.URL)
	if err != nil {
		add("oauth_grant_client_registration", false)
		return
	}
	redirect := "https://connector.example/callback"
	client, err := oauth.register(oauthClientFixture{Name: "Bridge Test connector", RedirectURIs: []string{redirect}, GrantTypes: []string{"authorization_code", "refresh_token"}})
	add("oauth_grant_client_registration", err == nil && client.ID != "")
	if err != nil {
		return
	}
	verifier := strings.Repeat("a", 43)
	challenge := sha256.Sum256([]byte(verifier))
	resource := w.Gateway.URL + "/mcp"
	issue := func() (oauthTokensFixture, string, error) {
		code, err := oauth.issue(client.ID, redirect, base64.RawURLEncoding.EncodeToString(challenge[:]), resource)
		if err != nil {
			return oauthTokensFixture{}, "", err
		}
		_, wrong := oauth.exchange("authorization_code", client.ID, code, redirect, strings.Repeat("b", 43), resource, "", "")
		add("oauth_grant_wrong_pkce_rejected", wrong != nil)
		token, err := oauth.exchange("authorization_code", client.ID, code, redirect, verifier, resource, "", "")
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
		Access  []oauthAccessFixture  `json:"oauth_access"`
		Refresh []oauthRefreshFixture `json:"oauth_refresh"`
	}
	json.Unmarshal(stored, &data)
	accessTTL, refreshTTL := false, false
	for _, v := range data.Access {
		if v.Hash == fixtureTokenHash([]byte(tokens.AccessToken)) {
			accessTTL = time.Until(v.ExpiresAt) > 59*time.Minute && time.Until(v.ExpiresAt) <= time.Hour
		}
	}
	for _, v := range data.Refresh {
		if v.Hash == fixtureTokenHash([]byte(tokens.RefreshToken)) {
			refreshTTL = time.Until(v.ExpiresAt) > 89*24*time.Hour && time.Until(v.ExpiresAt) <= 90*24*time.Hour
		}
	}
	add("oauth_grant_lifetimes", err == nil && accessTTL && refreshTTL)
	add("oauth_grant_secrets_hashed", !strings.Contains(string(stored), tokens.AccessToken) && !strings.Contains(string(stored), tokens.RefreshToken))
	rotated, err := oauth.exchange("refresh_token", client.ID, "", "", "", resource, tokens.RefreshToken, "")
	add("oauth_grant_refresh_rotates", err == nil && rotated.RefreshToken != tokens.RefreshToken && rotated.AccessToken != tokens.AccessToken && read(rotated.AccessToken) == 200)
	_, err = oauth.exchange("refresh_token", client.ID, "", "", "", resource, tokens.RefreshToken, "")
	add("oauth_grant_refresh_reuse_revokes", err != nil && read(rotated.AccessToken) == 401)
	_, err = oauth.exchange("refresh_token", client.ID, "", "", "", resource, rotated.RefreshToken, "")
	add("oauth_grant_refresh_family_revoked", err != nil)
	tokens, code, err := issue()
	_, replay := oauth.exchange("authorization_code", client.ID, code, redirect, verifier, resource, "", "")
	add("oauth_grant_code_reuse_revokes", err == nil && replay != nil && read(tokens.AccessToken) == 401)
	tokens, _, err = issue()
	principal, authErr := binaryOAuthPrincipal(ctx, w, w.Gateway.URL, tokens.AccessToken)
	_, revokeErr := w.GatewayCommand("agent", "revoke", principal)
	add("oauth_grant_owner_revocation", err == nil && authErr == nil && revokeErr == nil && read(tokens.AccessToken) == 401)
}
