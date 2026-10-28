package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	_ "github.com/JohanSardar/apple-bridge/internal/sourcefixture"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/url"
	"os/exec"
	"strings"
	"time"
)

func runSourceGrants(ctx context.Context, w *world.World, add func(string, bool)) {
	for _, path := range []string{"/.well-known/oauth-protected-resource", "/.well-known/oauth-protected-resource/mcp", "/.well-known/oauth-authorization-server"} {
		status, raw, _ := agentRequest(ctx, w.Gateway.URL, "", "GET", path, nil)
		add("dummy_grants_"+path, status == 200 && bytes.Contains(raw, []byte("labdummy:read")) && bytes.Contains(raw, []byte("labdummy:write")))
	}
	authority, err := trust.Open(w.Gateway.Home)
	add("grants_authority", err == nil)
	if err != nil {
		return
	}
	cookie, _, err := authority.CreateDeviceSession("trusted")
	owner := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: cookie}}
	resp, raw, err := owner.request("GET", "grants", nil)
	add("agent_ui_dummy_group", err == nil && resp.StatusCode == 200 && bytes.Contains(raw, []byte("Lab dummy")) && bytes.Contains(raw, []byte("labdummy:write")))
	client, err := authority.RegisterOAuth(trust.OAuthClient{Name: "Bridge Test grants", RedirectURIs: []string{"https://connector.example/callback"}})
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
	command := exec.CommandContext(ctx, "pnpm", "exec", "node", "dev/source-grants.mjs", owner.origin, query.Encode())
	command.Dir = "web"
	command.Env = append(command.Environ(), "AB_GRANTS_SESSION="+cookie)
	output, err := command.CombinedOutput()
	add("consent_and_agent_render_dummy_group", err == nil && bytes.Contains(output, []byte("source grants rendered")))
	if err != nil {
		println(string(output))
	}
	store, err := state.Open(w.Gateway.Home)
	add("dummy_store", err == nil)
	if err != nil {
		return
	}
	defer store.Close()
	_, err = store.CommitCapture(ctx, contracts.Capture{Source: "labdummy", Generation: "lab-grants", Checkpoint: "1", Observations: []contracts.Observation{{Kind: "upsert", Collection: "labdummy.item", NativeKey: "fixture", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"title":"Bridge Test grants"}`)}, {Kind: "upsert", Collection: "system.source", NativeKey: "labdummy", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"source":"labdummy","state":"ready"}`)}}})
	add("dummy_record_captured", err == nil)
	for _, scope := range []string{"messages:read", "labdummy:read", "labdummy:write"} {
		_, token, err := authority.CreateAgent("Bridge Test grants", trust.AgentApprove, []contracts.Grant{contracts.Grant(scope)})
		status, raw, _ := agentRequest(ctx, w.Gateway.URL, token, "POST", "/v1/op/records.query", contracts.ReadQuery{Collection: "labdummy.item"})
		var result contracts.ReadResult
		json.Unmarshal(raw, &result)
		expected := 1
		if scope == "messages:read" {
			expected = 0
		}
		add("dummy_visibility_"+scope, err == nil && status == 200 && len(result.Records) == expected)
	}
	ownerToken, tokenErr := authority.CreateToken([]contracts.Grant{"admin"})
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
		err := exec.CommandContext(ctx, "bin/bridge", "agent", "create", "--home", w.Gateway.Home, "--name", "Bridge Test CLI grants", "--scope", scope).Run()
		add("cli_grant_"+scope, (err == nil) == (scope != "labdummy:unknown"))
	}
}
