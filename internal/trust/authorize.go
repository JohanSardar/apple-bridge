package trust

import "github.com/JohanSardar/apple-bridge/internal/contracts"

func (a *Authority) Authorize(p Principal, grant contracts.Grant) error {
	return a.update(func(d *credentials) error {
		for _, current := range d.Principals {
			if principalActive(current) && !(current.Kind == "agent" && current.Level != AgentFull && len(grant) >= 6 && string(grant[len(grant)-6:]) == ":write") && (current.ID == p.ID || (p.Kind == "delegated" && current.Kind == "replica" && current.Fingerprint == p.Fingerprint && Subset(p.Grants, current.Grants))) && Subset([]contracts.Grant{grant}, current.Grants) {
				return nil
			}
		}
		return contracts.ErrUnauthorized
	})
}
