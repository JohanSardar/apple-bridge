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
		if len(parts) != 2 || parts[0] == "system" || parts[1] != "read" && parts[1] != "write" {
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
