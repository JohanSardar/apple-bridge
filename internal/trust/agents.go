package trust

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"strings"
	"time"
)

type AgentLevel string

const (
	AgentRead    AgentLevel = "read"
	AgentApprove AgentLevel = "approve"
	AgentFull    AgentLevel = "full"
)

type Agent struct {
	ID        string            `json:"id"`
	Name      string            `json:"name"`
	Level     AgentLevel        `json:"level"`
	Scopes    []contracts.Grant `json:"scopes"`
	CreatedAt time.Time         `json:"created_at"`
	RevokedAt *time.Time        `json:"revoked_at,omitempty"`
	LastUsed  *time.Time        `json:"last_used,omitempty"`
}

func principalActive(p Principal) bool {
	return p.RevokedAt == nil && (p.ExpiresAt.IsZero() || time.Now().Before(p.ExpiresAt))
}
func agentSummary(p Principal) Agent {
	return Agent{p.ID, p.Name, p.Level, p.Grants, p.CreatedAt, p.RevokedAt, p.LastUsed}
}
func AgentGrants(level AgentLevel, scopes []contracts.Grant) ([]contracts.Grant, error) {
	if level != "read" && level != "approve" && level != "full" || len(scopes) == 0 {
		return nil, contracts.ErrInvalid
	}
	out := []contracts.Grant{}
	for _, g := range scopes {
		parts := strings.Split(string(g), ":")
		group, known := contracts.GrantsForSource(contracts.Source(parts[0]))
		if !known || !group.Agent || len(parts) != 2 || parts[1] != "read" && parts[1] != "write" {
			return nil, contracts.ErrInvalid
		}
		if _, err := ParseGrants(string(g)); err != nil {
			return nil, err
		}
		if level == AgentRead {
			g = contracts.Grant(parts[0] + ":read")
		}
		out = append(out, g)
	}
	return out, nil
}
func (a *Authority) CreateAgent(name string, level AgentLevel, scopes []contracts.Grant) (Agent, string, error) {
	if level == "" {
		level = AgentApprove
	}
	name = strings.TrimSpace(name)
	if name == "" || len(name) > 120 {
		return Agent{}, "", contracts.ErrInvalid
	}
	grants, err := AgentGrants(level, scopes)
	if err != nil {
		return Agent{}, "", err
	}
	token, err := randomWebToken(32)
	if err != nil {
		return Agent{}, "", err
	}
	token = "aba-" + token
	p := Principal{ID: "agent:" + contracts.NewID(""), Kind: "agent", Name: name, Level: level, Grants: grants, CreatedAt: time.Now().UTC(), TokenHash: Fingerprint([]byte(token)), Revision: 1}
	err = a.update(func(d *credentials) error {
		if d.Enrollment != nil && !Subset(grants, d.Enrollment.Grants) {
			return contracts.ErrUnauthorized
		}
		d.Principals = append(d.Principals, p)
		return nil
	})
	if err != nil {
		return Agent{}, "", err
	}
	return agentSummary(p), token, nil
}
func (a *Authority) Agents() ([]Agent, error) {
	d, err := a.load()
	if err != nil {
		return nil, err
	}
	out := []Agent{}
	for _, p := range d.Principals {
		if p.Kind == "agent" {
			out = append(out, agentSummary(p))
		}
	}
	return out, nil
}

func IsAgentPrincipalID(id string) bool {
	if parent, local, delegated := strings.Cut(id, "/"); delegated {
		if !strings.HasPrefix(parent, "replica:") {
			return false
		}
		id = local
	}
	return strings.HasPrefix(id, "agent:") && len(id) > len("agent:") && !strings.ContainsAny(id, "/\r\n")
}
func (p Principal) IsAgent() bool {
	return (p.Kind == "agent" || p.Kind == "delegated") && IsAgentPrincipalID(p.ID)
}
