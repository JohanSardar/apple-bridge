package web

import (
	"crypto/rand"
	"encoding/base64"
	"fmt"
	"math/big"
	"net/url"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type ConnectorApproval struct {
	ID           string            `json:"id"`
	Code         string            `json:"code"`
	ClientName   string            `json:"client_name"`
	RedirectHost string            `json:"redirect_host"`
	Scopes       []contracts.Grant `json:"scopes"`
	ExpiresAt    time.Time         `json:"expires_at"`
}

type pendingConnector struct {
	ConnectorApproval
	client   trust.OAuthClient
	oauth    trust.OAuthCode
	state    string
	origin   string
	redirect string
	decided  bool
	attempts int
}

type ConnectorApprovals struct {
	mu        sync.Mutex
	pending   map[string]*pendingConnector
	Authority *trust.Authority
	Now       func() time.Time
	Wake      func()
}

func NewConnectorApprovals(authority *trust.Authority) *ConnectorApprovals {
	return &ConnectorApprovals{Authority: authority, pending: map[string]*pendingConnector{}, Now: time.Now}
}

func (a *ConnectorApprovals) prune() {
	for id, p := range a.pending {
		if !a.Now().Before(p.ExpiresAt) {
			delete(a.pending, id)
		}
	}
}

func (a *ConnectorApprovals) begin(client trust.OAuthClient, code trust.OAuthCode, state, origin string, scopes []contracts.Grant) (ConnectorApproval, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.prune()
	if len(a.pending) >= 128 {
		return ConnectorApproval{}, contracts.ErrUnavailable
	}
	raw := make([]byte, 32)
	if _, err := rand.Read(raw); err != nil {
		return ConnectorApproval{}, err
	}
	var matching string
	for {
		n, err := rand.Int(rand.Reader, big.NewInt(10000))
		if err != nil {
			return ConnectorApproval{}, err
		}
		matching = fmt.Sprintf("%04d", n.Int64())
		duplicate := false
		for _, p := range a.pending {
			duplicate = duplicate || p.Code == matching
		}
		if !duplicate {
			break
		}
	}
	dest, err := url.Parse(code.RedirectURI)
	if err != nil {
		return ConnectorApproval{}, err
	}
	summary := ConnectorApproval{ID: "connector:" + base64.RawURLEncoding.EncodeToString(raw), Code: matching, ClientName: client.Name, RedirectHost: dest.Host, Scopes: scopes, ExpiresAt: a.Now().Add(5 * time.Minute)}
	a.pending[summary.ID] = &pendingConnector{ConnectorApproval: summary, client: client, oauth: code, state: state, origin: origin}
	if a.Wake != nil {
		a.Wake()
	}
	return summary, nil
}

func (a *ConnectorApprovals) Pending() []ConnectorApproval {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.prune()
	out := []ConnectorApproval{}
	for _, p := range a.pending {
		if !p.decided {
			out = append(out, p.ConnectorApproval)
		}
	}
	return out
}

func (a *ConnectorApprovals) Decide(id, code string, approve bool) error {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.prune()
	p := a.pending[id]
	if p == nil || p.decided || p.attempts >= 5 {
		return contracts.ErrUnauthorized
	}
	p.attempts++
	if p.Code != code {
		return contracts.ErrUnauthorized
	}
	decision := "reject"
	if approve {
		decision = "approve"
	}
	redirect, err := issueConnectorDecision(a.Authority, p.client, p.oauth, p.state, p.origin, decision, trust.AgentApprove, p.Scopes)
	if err != nil {
		return err
	}
	p.redirect, p.decided = redirect, true
	return nil
}

func (a *ConnectorApprovals) poll(id, origin string) (string, error) {
	a.mu.Lock()
	defer a.mu.Unlock()
	a.prune()
	p := a.pending[id]
	if p == nil || p.origin != origin {
		return "", contracts.ErrUnauthorized
	}
	if !p.decided {
		return "", nil
	}
	delete(a.pending, id)
	return p.redirect, nil
}

func issueConnectorDecision(authority *trust.Authority, client trust.OAuthClient, code trust.OAuthCode, state, origin, decision string, level trust.AgentLevel, scopes []contracts.Grant) (string, error) {
	dest, err := url.Parse(code.RedirectURI)
	if err != nil {
		return "", err
	}
	q := dest.Query()
	q.Set("state", state)
	q.Set("iss", origin)
	if decision == "reject" {
		q.Set("error", "access_denied")
	} else {
		agent, _, err := authority.CreateAgent(client.Name, level, scopes)
		if err != nil {
			return "", err
		}
		code.Principal = agent.ID
		raw, err := authority.IssueOAuthCode(code)
		if err != nil {
			authority.Revoke(agent.ID)
			return "", err
		}
		q.Set("code", raw)
	}
	dest.RawQuery = q.Encode()
	return dest.String(), nil
}
