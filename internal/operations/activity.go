package operations

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"time"
)

func (g *LocalGateway) recordAgentCommand(r contracts.CommandRecord) error {
	if !trust.IsAgentPrincipalID(r.Principal) {
		return nil
	}
	id := r.Command.ID
	if id == "" {
		id = contracts.NewCommandID()
	}
	return g.State.RecordAgentActivity(g.Context, state.AgentActivity{Sequence: r.Cursor.Sequence, ID: r.Principal + ":" + id, Principal: r.Principal, Operation: r.Command.Op, Kind: "command", State: string(r.State), At: time.Now().UTC(), ArgumentBytes: len(r.Command.Args), ArgumentHash: trust.Fingerprint(r.Command.Args)})
}
func (s *Service) MaintainAgentAccess(ctx context.Context) {
	tick := time.NewTicker(time.Minute)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case <-tick.C:
			s.State.PruneAgentActivity(ctx)
			if g, ok := s.Target.(*LocalGateway); ok {
				g.Approvals(ctx)
			}
		}
	}
}
