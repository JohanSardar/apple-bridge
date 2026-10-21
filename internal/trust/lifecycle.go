package trust

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (a *Authority) load() (credentials, error) {
	var d credentials
	b, err := os.ReadFile(filepath.Join(a.home, "credentials.json"))
	if err != nil {
		return d, err
	}
	err = json.Unmarshal(b, &d)
	return d, err
}
func (a *Authority) Principal(id string) (Principal, error) {
	d, err := a.load()
	if err != nil {
		return Principal{}, err
	}
	for _, p := range d.Principals {
		if p.ID == id && principalActive(p) {
			return p, nil
		}
	}
	return Principal{}, contracts.ErrUnauthorized
}
func (a *Authority) Revoke(id string) error {
	return a.update(func(d *credentials) error {
		for i, p := range d.Principals {
			if p.ID == id {
				if p.Kind == "agent" {
					now := time.Now().UTC()
					d.Principals[i].RevokedAt = &now
					d.Principals[i].TokenHash = ""
					d.Principals[i].Revision++
					return nil
				}
				d.Principals = append(d.Principals[:i], d.Principals[i+1:]...)
				return nil
			}
		}
		return contracts.ErrNotFound
	})
}
func (a *Authority) ChangeScope(id string, grants []contracts.Grant) error {
	return a.update(func(d *credentials) error {
		for i, p := range d.Principals {
			if p.ID == id && p.Kind == "replica" {
				d.Principals[i].Grants = grants
				d.Principals[i].Revision++
				return nil
			}
		}
		return contracts.ErrNotFound
	})
}
func (a *Authority) WatchPrincipal(ctx context.Context, p Principal) (context.Context, context.CancelFunc) {
	ctx, cancel := context.WithCancel(ctx)
	go func() {
		defer cancel()
		tick := time.NewTicker(100 * time.Millisecond)
		defer tick.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-tick.C:
				current, err := a.Principal(p.ID)
				if err != nil || current.Revision != p.Revision {
					return
				}
			}
		}
	}()
	return ctx, cancel
}

func EffectiveGrants(grants, ceiling []contracts.Grant) []contracts.Grant {
	result := []contracts.Grant{}
	for _, grant := range grants {
		if Subset([]contracts.Grant{grant}, ceiling) {
			result = append(result, grant)
		} else if strings.HasSuffix(string(grant), ":write") {
			read := contracts.Grant(strings.TrimSuffix(string(grant), ":write") + ":read")
			if Subset([]contracts.Grant{read}, ceiling) {
				result = append(result, read)
			}
		}
	}
	return result
}
