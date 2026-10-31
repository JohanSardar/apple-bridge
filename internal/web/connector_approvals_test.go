package web

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"io"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func connectorFixture(t *testing.T) (*ConnectorApprovals, trust.OAuthClient, trust.OAuthCode) {
	t.Helper()
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	authority, err := trust.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	client, err := authority.RegisterOAuth(trust.OAuthClient{Name: "[ab-test] connector", RedirectURIs: []string{"https://connector.example/callback"}, GrantTypes: []string{"authorization_code", "refresh_token"}})
	if err != nil {
		t.Fatal(err)
	}
	challenge := sha256.Sum256([]byte(strings.Repeat("a", 43)))
	code := trust.OAuthCode{ClientID: client.ID, RedirectURI: client.RedirectURIs[0], Challenge: base64.RawURLEncoding.EncodeToString(challenge[:]), Resource: "http://localhost:7780/mcp"}
	return NewConnectorApprovals(authority), client, code
}

func TestConnectorDecisionAndPollAreSingleUseUnderConcurrency(t *testing.T) {
	approvals, client, code := connectorFixture(t)
	p, err := approvals.begin(client, code, "state", "http://localhost:7780", []contracts.Grant{"messages:read"})
	if err != nil {
		t.Fatal(err)
	}
	var decisions, polls atomic.Int32
	var group sync.WaitGroup
	for i := 0; i < 20; i++ {
		group.Go(func() {
			if approvals.Decide(p.ID, p.Code, true) == nil {
				decisions.Add(1)
			}
		})
	}
	group.Wait()
	if decisions.Load() != 1 {
		t.Fatalf("accepted %d decisions", decisions.Load())
	}
	if _, err := approvals.poll(p.ID, "https://attacker.example"); err == nil {
		t.Fatal("wrong origin read redirect")
	}
	for i := 0; i < 20; i++ {
		group.Go(func() {
			if redirect, err := approvals.poll(p.ID, "http://localhost:7780"); err == nil && redirect != "" {
				polls.Add(1)
			}
		})
	}
	group.Wait()
	agents, _ := approvals.Authority.Agents()
	if polls.Load() != 1 || len(agents) != 1 {
		t.Fatalf("polls=%d agents=%d", polls.Load(), len(agents))
	}
}

func TestConnectorExpiryAndAttemptLimit(t *testing.T) {
	approvals, client, code := connectorFixture(t)
	now := time.Now()
	approvals.Now = func() time.Time { return now }
	for _, expired := range []bool{false, true} {
		p, err := approvals.begin(client, code, "state", "http://localhost:7780", []contracts.Grant{"messages:read"})
		if err != nil {
			t.Fatal(err)
		}
		if expired {
			now = now.Add(5 * time.Minute)
		} else {
			for i := 0; i < 5; i++ {
				if approvals.Decide(p.ID, "wrong", true) == nil {
					t.Fatal("wrong code accepted")
				}
			}
		}
		if approvals.Decide(p.ID, p.Code, true) == nil {
			t.Fatal("expired or exhausted request accepted")
		}
	}
	agents, _ := approvals.Authority.Agents()
	if len(agents) != 0 {
		t.Fatal("invalid approval created an agent")
	}
}

func TestConnectorPublicRoutesCannotApproveOrLeakLocalRequests(t *testing.T) {
	approvals, client, code := connectorFixture(t)
	handler := Handler(Deps{Authority: approvals.Authority, ConnectorApprovals: approvals, Address: "127.0.0.1:7780"})
	q := url.Values{"client_id": {client.ID}, "response_type": {"code"}, "redirect_uri": {code.RedirectURI}, "code_challenge": {code.Challenge}, "code_challenge_method": {"S256"}, "resource": {code.Resource}, "scope": {"messages:read"}}
	q.Set("resource", "http://127.0.0.1:7780/mcp")
	for _, test := range []struct {
		method, path, origin, body string
		status                     int
	}{
		{"POST", "/oauth/authorize?" + q.Encode(), "http://127.0.0.1:7780", `{"decision":"approve","scopes":["messages:read"]}`, 401},
		{"POST", "/oauth/pending?" + q.Encode(), "https://attacker.example", "{}", 403},
		{"GET", "/pending", "", "", 200},
		{"GET", "/oauth/pending/unknown", "", "", 410},
	} {
		req := httptest.NewRequest(test.method, "http://127.0.0.1:7780"+test.path, strings.NewReader(test.body))
		if test.origin != "" {
			req.Header.Set("Origin", test.origin)
		}
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, req)
		if response.Code != test.status {
			t.Fatalf("%s: got %d", test.path, response.Code)
		}
		if strings.Contains(response.Body.String(), `"client_name"`) {
			t.Fatal("public route listed local pending approvals")
		}
	}
	home := t.TempDir()
	closeSocket, err := ServeConnectorApprovals(home, approvals)
	if err != nil {
		t.Fatal(err)
	}
	defer closeSocket()
	p, err := approvals.begin(client, code, "state", "http://localhost:7780", []contracts.Grant{"messages:read"})
	if err != nil {
		t.Fatal(err)
	}
	if err := ConnectorCommand(context.Background(), home, "deny", p.ID, p.Code, io.Discard); err != nil {
		t.Fatal(err)
	}
	redirect, err := approvals.poll(p.ID, "http://localhost:7780")
	if err != nil || !strings.Contains(redirect, "access_denied") {
		t.Fatal("local denial failed")
	}
}
