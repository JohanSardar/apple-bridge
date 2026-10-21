package operations

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"time"
)

func (g *LocalGateway) expireApproval(ctx context.Context, r contracts.CommandRecord) (contracts.CommandRecord, error) {
	if r.State == contracts.CommandAwaitingApproval && !gatewayNow().Before(r.ExpiresAt) {
		return g.transition(ctx, r, contracts.CommandExpired, contracts.DispositionExpired, nil, nil)
	}
	return r, nil
}
func (g *LocalGateway) Approvals(ctx context.Context) ([]contracts.CommandRecord, error) {
	rows, err := g.State.PendingApprovals(ctx)
	if err != nil {
		return nil, err
	}
	out := []contracts.CommandRecord{}
	for _, r := range rows {
		if r.State != contracts.CommandAwaitingApproval {
			continue
		}
		r, err = g.expireApproval(ctx, r)
		if err != nil && r.State == contracts.CommandAwaitingApproval {
			return nil, err
		}
		if r.State == contracts.CommandAwaitingApproval {
			out = append(out, r)
		}
	}
	return out, nil
}
func (g *LocalGateway) DecideApproval(ctx context.Context, owner trust.Principal, id string, approve bool) (contracts.CommandRecord, error) {
	if owner.Kind != "device" || g.Authority.Authorize(owner, "admin") != nil {
		return contracts.CommandRecord{}, contracts.ErrUnauthorized
	}
	unlock, err := g.lock(ctx, "command:"+id)
	if err != nil {
		return contracts.CommandRecord{}, err
	}
	defer unlock()
	r, err := g.State.Command(ctx, id)
	if err != nil {
		return r, err
	}
	if r.State != contracts.CommandAwaitingApproval {
		return r, nil
	}
	r, err = g.expireApproval(ctx, r)
	if err != nil || r.State != contracts.CommandAwaitingApproval {
		return r, err
	}
	if !approve {
		return g.transition(ctx, r, contracts.CommandRejectedByOwner, contracts.DispositionRejectedByOwner, nil, nil)
	}
	p, err := g.Authority.Principal(r.Principal)
	if err != nil {
		return g.transition(ctx, r, contracts.CommandRejected, contracts.DispositionRejectedBeforeDispatch, nil, contracts.ErrUnauthorized)
	}
	if err = g.authorize(p, r.Command.Op); err != nil {
		return g.transition(ctx, r, contracts.CommandRejected, contracts.DispositionRejectedBeforeDispatch, nil, contracts.ErrUnauthorized)
	}
	if !g.Peer.Available() {
		return r, contracts.ErrUnavailable
	}
	era, err := g.State.AdmissionEra(ctx)
	if err != nil {
		return r, err
	}
	if era != r.AdmissionEra {
		return g.transition(ctx, r, contracts.CommandExpired, contracts.DispositionExpired, nil, nil)
	}
	ttl, _ := contracts.CommandTTL(r.Command.Op)
	if !r.Command.ExpiresAt.IsZero() {
		ttl = r.Command.ExpiresAt.Sub(r.CreatedAt)
	}
	r.ExpiresAt = gatewayNow().Add(ttl)
	r, err = g.transition(ctx, r, contracts.CommandAdmitted, "", nil, nil)
	if err != nil {
		return r, err
	}
	args, err := contracts.ValidateCommand(r.Command)
	if err != nil {
		return g.transition(ctx, r, contracts.CommandRejected, contracts.DispositionRejectedBeforeDispatch, nil, contracts.ErrInvalid)
	}
	go g.execute(r, p, r.Command, args, r.ExpiresAt, time.Now().Add(ttl), r.RequestHash)
	return r, nil
}
