package operations

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
)

func (g *LocalGateway) ReauthorizeIPhone(ctx context.Context, r contracts.CommandRecord) error {
	var p trust.Principal
	var err error
	if trust.IsAgentPrincipalID(r.Principal) && strings.Contains(r.Principal, "/") {
		if g.Reauthorize == nil {
			return contracts.ErrUnauthorized
		}
		p, err = g.Reauthorize(ctx, r.Principal, r.Command.ID, r.RequestHash)
	} else {
		p, err = g.Authority.Principal(r.Principal)
	}
	if err != nil {
		return err
	}
	return g.authorize(p, r.Command.Op)
}
