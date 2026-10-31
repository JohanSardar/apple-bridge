package scenarios

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func securityOwnerRequests(ctx context.Context, c *webClient, add func(string, bool)) {
	for _, path := range []string{"/oauth/authorize", "/web/v1/approvals/test", "/web/v1/passkeys/options", "/web/v1/agents"} {
		for _, origin := range []string{"", "null", "https://attacker.example"} {
			req, _ := http.NewRequestWithContext(ctx, "POST", c.origin+path, strings.NewReader(`{"decision":"approve"}`))
			req.AddCookie(c.cookie)
			if origin != "" {
				req.Header.Set("Origin", origin)
			}
			response, err := http.DefaultClient.Do(req)
			denied := err == nil && response.StatusCode == 403
			if response != nil {
				response.Body.Close()
			}
			add("approve_csrf_and_cross_origin_refused", denied)
		}
	}
	response, body, err := c.request("GET", "session", nil)
	safe := err == nil && response.StatusCode == 200 && !strings.Contains(string(body), c.cookie.Value)
	if response != nil && strings.HasPrefix(c.origin, "https:") {
		cookies := response.Cookies()
		safe = safe && len(cookies) == 1
		if len(cookies) == 1 {
			cookie := cookies[0]
			safe = safe && cookie.Secure && cookie.HttpOnly && cookie.Path == "/" && cookie.Domain == "" && cookie.SameSite == http.SameSiteStrictMode
		}
	}
	add("session_cookie_flags_and_response_redaction", safe)
}

func securityTrustBoundaries(w *world.World, a *trust.Authority, add func(string, bool)) {
	token, session, err := a.CreateDeviceSession("public")
	p, _, authErr := a.AuthenticateDevice(token)
	expired := editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, v := range data["devices"].([]any) {
			if v.(map[string]any)["device_id"] == session.DeviceID {
				v.(map[string]any)["expires_at"] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
			}
		}
	})
	add("expired_owner_session_cannot_enroll_passkey", err == nil && authErr == nil && expired == nil && a.SavePasskey(trust.Passkey{ID: "[ab-test]", RPID: "localhost"}, "", &p) != nil)
	agent, _, err := a.CreateAgent("[ab-test] expired", trust.AgentRead, []contracts.Grant{"messages:read"})
	expired = editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		for _, v := range data["principals"].([]any) {
			if v.(map[string]any)["id"] == agent.ID {
				v.(map[string]any)["expires_at"] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
			}
		}
		data["oauth_refresh"] = append(data["oauth_refresh"].([]any), map[string]any{"hash": trust.Fingerprint([]byte("[ab-test] refresh")), "principal": agent.ID, "client_id": "client", "resource": "resource", "expires_at": time.Now().Add(time.Hour).Format(time.RFC3339Nano)})
	})
	_, exchangeErr := a.ExchangeOAuth("refresh_token", "client", "", "", "", "resource", "[ab-test] refresh", "")
	add("expired_agent_cannot_mint_oauth_token", err == nil && expired == nil && exchangeErr != nil)
	replica := trust.Principal{ID: "replica:[ab-test]", Kind: "replica", Fingerprint: a.Fingerprint, Grants: []contracts.Grant{"messages:read", "contacts:read"}}
	err = editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
		raw, _ := json.Marshal(replica)
		var entry map[string]any
		json.Unmarshal(raw, &entry)
		data["principals"] = append(data["principals"].([]any), entry)
	})
	delegated := trust.Principal{ID: replica.ID + "/agent:test", Kind: "delegated", Level: trust.AgentRead, Fingerprint: a.Fingerprint, Grants: []contracts.Grant{"messages:read"}}
	add("delegated_scope_cannot_expand_to_replica_scope", err == nil && a.Authorize(delegated, "messages:read") == nil && a.Authorize(delegated, "contacts:read") != nil)
	cert, err := x509.ParseCertificate(a.Certificate.Certificate[0])
	if err != nil {
		add("replica_certificate_fixture", false)
		return
	}
	for _, field := range []string{"expires_at", "revoked_at"} {
		err = editAgentCredentials(w.Gateway.Home, func(data map[string]any) {
			for _, v := range data["principals"].([]any) {
				if v.(map[string]any)["id"] == replica.ID {
					v.(map[string]any)[field] = time.Now().Add(-time.Second).Format(time.RFC3339Nano)
				}
			}
		})
		_, authErr = a.AuthenticateCertificate(cert)
		add("inactive_replica_certificate_refused", err == nil && authErr != nil)
	}
}
