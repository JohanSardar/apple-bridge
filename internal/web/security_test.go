package web

import (
	"crypto/tls"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"
)

func TestOwnerGateOriginsAndCookies(t *testing.T) {
	approvals, client, code := connectorFixture(t)
	token, session, err := approvals.Authority.CreateDeviceSession("trusted")
	if err != nil {
		t.Fatal(err)
	}
	origin := "https://bridge.example"
	code.Resource = origin + "/mcp"
	handler := Handler(Deps{Authority: approvals.Authority, ConnectorApprovals: approvals, PublicHost: func() string { return "bridge.example" }})
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {code.RedirectURI}, "code_challenge": {code.Challenge}, "code_challenge_method": {"S256"}, "resource": {code.Resource}, "scope": {"messages:read"}}
	for _, path := range []string{"/oauth/authorize?" + q.Encode(), "/web/v1/agents", "/web/v1/approvals/test", "/web/v1/passkeys/options"} {
		for _, source := range []string{"", "null", "https://attacker.example", origin} {
			for _, authenticated := range []bool{false, true} {
				req := httptest.NewRequest("POST", origin+path, strings.NewReader(`{"decision":"approve","scopes":["messages:read"]}`))
				req.TLS = &tls.ConnectionState{}
				if source != "" {
					req.Header.Set("Origin", source)
				}
				if authenticated {
					req.AddCookie(&http.Cookie{Name: SessionCookie, Value: token})
				}
				response := httptest.NewRecorder()
				handler.ServeHTTP(response, req)
				if source != origin && response.Code != 403 {
					t.Fatalf("CSRF accepted on %s", path)
				}
				if source == origin && !authenticated && response.Code != 401 {
					t.Fatalf("owner gate missing on %s", path)
				}
				if strings.Contains(response.Body.String(), token) {
					t.Fatal("session leaked into response")
				}
			}
		}
	}
	req := httptest.NewRequest("GET", origin+"/web/v1/session", nil)
	req.TLS = &tls.ConnectionState{}
	req.AddCookie(&http.Cookie{Name: SessionCookie, Value: token})
	response := httptest.NewRecorder()
	handler.ServeHTTP(response, req)
	cookies := response.Result().Cookies()
	if response.Code != 200 || len(cookies) != 1 {
		t.Fatal("session response failed")
	}
	c := cookies[0]
	if c.Name != SessionCookie || c.Domain != "" || c.Path != "/" || !c.Secure || !c.HttpOnly || c.SameSite != http.SameSiteStrictMode || c.MaxAge <= 0 || c.Expires.After(session.ExpiresAt.Add(time.Second)) {
		t.Fatal("unsafe session cookie")
	}
	if response.Header().Get("Cache-Control") != "no-store" || response.Header().Get("Referrer-Policy") != "no-referrer" {
		t.Fatal("unsafe response caching")
	}
	req.Header["Origin"] = []string{origin, origin}
	req.Method = "POST"
	response = httptest.NewRecorder()
	handler.ServeHTTP(response, req)
	if response.Code != 403 {
		t.Fatal("duplicate origin accepted")
	}
}

func TestOAuthRedirectAndPKCEDowngrade(t *testing.T) {
	approvals, client, code := connectorFixture(t)
	origin := "http://127.0.0.1:7780"
	handler := Handler(Deps{Authority: approvals.Authority, Address: "127.0.0.1:7780"})
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {code.RedirectURI}, "code_challenge": {code.Challenge}, "code_challenge_method": {"S256"}, "resource": {origin + "/mcp"}}
	for _, test := range []struct{ key, value string }{
		{"redirect_uri", "https://attacker.example/callback"},
		{"redirect_uri", code.RedirectURI + "/../escape"},
		{"redirect_uri", "//attacker.example"},
		{"redirect_uri", "javascript:alert(1)"},
		{"redirect_uri", code.RedirectURI + "#fragment"},
		{"code_challenge_method", "plain"},
		{"code_challenge_method", ""},
		{"code_challenge", ""},
		{"resource", "https://attacker.example/mcp"},
	} {
		bad := url.Values{}
		for k, v := range q {
			bad[k] = append([]string{}, v...)
		}
		bad.Set(test.key, test.value)
		req := httptest.NewRequest("GET", origin+"/oauth/authorize?"+bad.Encode(), nil)
		req.RemoteAddr = "127.0.0.1:12345"
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, req)
		if response.Code != 400 || response.Header().Get("Location") != "" {
			t.Fatalf("invalid authorization accepted for %s", test.key)
		}
	}
}
