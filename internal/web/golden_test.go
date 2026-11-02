package web

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"

	"github.com/JohanSardar/apple-bridge/contracts/golden"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func TestOAuthGolden(t *testing.T) {
	approvals, client, code := connectorFixture(t)
	authority := approvals.Authority
	_, _, e := authority.CreateAgent("[ab-test] synthetic", "read", []contracts.Grant{"messages:read"})
	if e != nil {
		t.Fatal(e)
	}
	token, _, e := authority.CreateDeviceSession("trusted")
	if e != nil {
		t.Fatal(e)
	}
	handler := Handler(Deps{Authority: authority, ConnectorApprovals: approvals, Address: "localhost:7780"})
	transcript := []any{}
	replacements := []string{client.ID, "client-fixture"}
	request := func(method, path, body string, form bool) *httptest.ResponseRecorder {
		req := httptest.NewRequest(method, "http://localhost:7780"+path, strings.NewReader(body))
		req.Header.Set("Origin", "http://localhost:7780")
		req.AddCookie(&http.Cookie{Name: LocalSessionCookie, Value: token})
		req.Header.Set("Content-Type", "application/json")
		if form {
			req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
		}
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, req)
		return response
	}
	record := func(method, path, body string, response *httptest.ResponseRecorder) {
		var value map[string]any
		if json.Unmarshal(response.Body.Bytes(), &value) == nil {
			for _, key := range []string{"access_token", "refresh_token"} {
				if s, ok := value[key].(string); ok && s != "" {
					replacements = append(replacements, s, key+"-fixture")
				}
			}
		}
		replace := strings.NewReplacer(replacements...)
		transcript = append(transcript, map[string]any{"method": method, "path": replace.Replace(path), "body": replace.Replace(body), "status": response.Code, "response": replace.Replace(response.Body.String())})
	}
	for _, path := range []string{"/.well-known/oauth-protected-resource", "/.well-known/oauth-authorization-server"} {
		r := request("GET", path, "", false)
		if r.Code != 200 {
			t.Fatal(r.Code)
		}
		record("GET", path, "", r)
	}
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {code.RedirectURI}, "scope": {"messages:read"}, "resource": {code.Resource}, "state": {"fixture"}, "code_challenge": {code.Challenge}, "code_challenge_method": {"S256"}}
	path := "/oauth/authorize?" + q.Encode()
	body := `{"decision":"approve","level":"read","scopes":["messages:read"]}`
	r := request("POST", path, body, false)
	if r.Code != 200 {
		t.Fatalf("consent %d %s", r.Code, r.Body.String())
	}
	var result struct {
		Redirect string `json:"redirect"`
	}
	json.Unmarshal(r.Body.Bytes(), &result)
	redirect, e := url.Parse(result.Redirect)
	if e != nil {
		t.Fatal(e)
	}
	issued := redirect.Query().Get("code")
	if issued == "" {
		t.Fatal("missing code")
	}
	replacements = append(replacements, issued, "code-fixture")
	record("POST", path, body, r)
	form := url.Values{"grant_type": {"authorization_code"}, "client_id": {client.ID}, "code": {issued}, "redirect_uri": {code.RedirectURI}, "code_verifier": {strings.Repeat("a", 43)}, "resource": {code.Resource}}
	r = request("POST", "/oauth/token", form.Encode(), true)
	if r.Code != 200 {
		t.Fatalf("token %d %s", r.Code, r.Body.String())
	}
	record("POST", "/oauth/token", form.Encode(), r)
	var tokens trust.OAuthTokens
	json.Unmarshal(r.Body.Bytes(), &tokens)
	form = url.Values{"grant_type": {"refresh_token"}, "client_id": {client.ID}, "refresh_token": {tokens.RefreshToken}, "resource": {code.Resource}}
	r = request("POST", "/oauth/token", form.Encode(), true)
	if r.Code != 200 {
		t.Fatalf("refresh %d %s", r.Code, r.Body.String())
	}
	record("POST", "/oauth/token", form.Encode(), r)
	b, e := json.MarshalIndent(map[string]any{"impl": "go", "normalization": "random client IDs, codes and tokens are replaced with named synthetic values; bodies retain wire formatting", "exchanges": transcript}, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	golden.Check(t, "oauth.json", append(b, '\n'))
}
