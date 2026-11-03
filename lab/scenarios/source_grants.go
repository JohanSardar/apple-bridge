package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"strings"
	"time"
)

func runSourceGrants(ctx context.Context, w *world.World, add func(string, bool)) {
	for _, path := range []string{"/.well-known/oauth-protected-resource", "/.well-known/oauth-protected-resource/mcp", "/.well-known/oauth-authorization-server"} {
		status, raw, _ := agentRequest(ctx, w.Gateway.URL, "", "GET", path, nil)
		add("dummy_grants_"+path, status == 200 && bytes.Contains(raw, []byte("labdummy:read")) && bytes.Contains(raw, []byte("labdummy:write")))
	}
	_, err := w.GatewayCommand("agent", "list")
	add("grants_authority", err == nil)
	if err != nil {
		return
	}
	oauth, err := newBinaryOAuthFixture(ctx, w, w.Gateway.URL)
	if err != nil {
		add("consent_dummy_group", false)
		return
	}
	owner := oauth.owner
	client, err := oauth.register(oauthClientFixture{Name: "Bridge Test grants", RedirectURIs: []string{"https://connector.example/callback"}})

	challenge := sha256.Sum256([]byte(strings.Repeat("a", 43)))
	query := url.Values{"client_id": {client.ID}, "redirect_uri": {"https://connector.example/callback"}, "response_type": {"code"}, "resource": {owner.origin + "/mcp"}, "code_challenge_method": {"S256"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(challenge[:])}, "scope": {"labdummy:read labdummy:write"}}
	req, _ := http.NewRequestWithContext(ctx, "GET", owner.origin+"/oauth/authorize?"+query.Encode()+"&format=json", nil)
	req.AddCookie(owner.cookie)
	response, err := http.DefaultClient.Do(req)
	if err == nil {
		var consent struct {
			Groups []contracts.SourceGrants `json:"groups"`
		}
		json.NewDecoder(response.Body).Decode(&consent)
		response.Body.Close()
		found := false
		for _, group := range consent.Groups {
			found = found || group.Source == "labdummy" && len(group.Grants) == 2
		}
		add("consent_dummy_group", response.StatusCode == 200 && found)
	} else {
		add("consent_dummy_group", false)
	}
	_, err = w.Records(w.Gateway)
	add("dummy_store", err == nil)
	if err != nil {
		return
	}
	err = w.ControlAction("/capture", contracts.Capture{Source: "labdummy", Generation: "lab-grants", Checkpoint: "1", Observations: []contracts.Observation{{Kind: "upsert", Collection: "labdummy.item", NativeKey: "fixture", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"title":"Bridge Test grants"}`)}, {Kind: "upsert", Collection: "system.source", NativeKey: "labdummy", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"source":"labdummy","state":"ready"}`)}}})
	add("dummy_record_captured", err == nil)
	for _, scope := range []string{"messages:read", "labdummy:read", "labdummy:write"} {
		token, err := binaryAgentToken(w, "Bridge Test grants", "approve", scope)
		status, raw, _ := agentRequest(ctx, w.Gateway.URL, token, "POST", "/v1/op/records.query", contracts.ReadQuery{Collection: "labdummy.item"})
		var result contracts.ReadResult
		json.Unmarshal(raw, &result)
		expected := 1
		if scope == "messages:read" {
			expected = 0
		}
		add("dummy_visibility_"+scope, err == nil && status == 200 && len(result.Records) == expected)
	}
	ownerToken, tokenErr := w.ScopedToken(w.Gateway, "admin")
	add("source_status_owner_token", tokenErr == nil)
	var statuses contracts.ReadResult
	status, raw, _ := agentRequest(ctx, w.Gateway.URL, ownerToken, "POST", "/v1/op/sources.status", map[string]any{})
	json.Unmarshal(raw, &statuses)
	found := false
	for _, record := range statuses.Records {
		if record.Source == "labdummy" {
			var data struct {
				Grants []contracts.Grant `json:"grants"`
			}
			json.Unmarshal(record.Data, &data)
			found = len(data.Grants) == 2
		}
	}
	add("source_status_dummy_grants", status == 200 && found)
	for _, scope := range []string{"labdummy:read", "labdummy:write", "labdummy:unknown"} {
		_, err := w.GatewayCommand("agent", "create", "--name", "Bridge Test CLI grants", "--scope", scope)
		add("cli_grant_"+scope, (err == nil) == (scope != "labdummy:unknown"))
	}
}
