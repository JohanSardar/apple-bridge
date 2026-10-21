package trust

import "github.com/JohanSardar/apple-bridge/internal/contracts"

func (a *Authority) Authorize(p Principal, grant contracts.Grant) error {
	return a.update(func(d *credentials) error {
		for _, current := range d.Principals {
			grants := current.Grants
			if current.Kind == "agent" && d.Enrollment != nil {
				grants = EffectiveGrants(grants, d.Enrollment.Grants)
			}
			if principalActive(current) && !(current.Kind == "agent" && current.Level == AgentRead && len(grant) >= 6 && string(grant[len(grant)-6:]) == ":write") && (current.ID == p.ID || (p.Kind == "delegated" && current.Kind == "replica" && current.Fingerprint == p.Fingerprint && Subset(p.Grants, current.Grants))) && Subset([]contracts.Grant{grant}, grants) {
				return nil
			}
		}
		return contracts.ErrUnauthorized
	})
}
