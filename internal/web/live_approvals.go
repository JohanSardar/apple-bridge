package web

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (l *liveSocket) emitApproval(command contracts.CommandRecord) {
	if !trust.IsAgentPrincipalID(command.Principal) || !l.identity.Principal.Visibility()[command.Source] || len(command.History) == 0 || command.History[0].State != contracts.CommandAwaitingApproval {
		return
	}
	l.send(map[string]any{"t": "approval", "command": map[string]any{"command_id": command.Command.ID}}, false)
}
func (l *liveSocket) emitApprovalMarker(record contracts.Record) {
	var marker struct {
		Principal        string `json:"principal"`
		AwaitingApproval bool   `json:"awaiting_approval"`
	}
	if record.Deleted || !l.identity.Principal.Visibility().Allows(record) || json.Unmarshal(record.Data, &marker) != nil || !trust.IsAgentPrincipalID(marker.Principal) || !marker.AwaitingApproval {
		return
	}
	l.send(map[string]any{"t": "approval", "command": map[string]any{"command_id": record.ID}}, false)
}
func (l *liveSocket) rereadApprovals() error {
	if _, local := l.server.deps.Operations.Target.(*operations.LocalGateway); local {
		commands, err := l.server.deps.Operations.State.PendingApprovals(l.ctx)
		if err != nil {
			return err
		}
		for _, command := range commands {
			l.emitApproval(command)
		}
		return nil
	}
	after := ""
	for {
		result, err := l.server.deps.Operations.State.PendingApprovalMarkers(l.ctx, l.identity.Principal.Visibility(), after)
		if err != nil {
			return err
		}
		for _, record := range result.Records {
			l.emitApprovalMarker(record)
		}
		if result.Next == "" {
			return nil
		}
		after = result.Next
	}
}
func (l *liveSocket) resetApprovals() (*state.ChangeIterator, error) {
	delay := 100 * time.Millisecond
	for l.ctx.Err() == nil {
		iterator, err := l.server.deps.Operations.Observe(l.ctx, l.identity.Principal, l.server.deps.Operations.Head())
		if err == nil {
			return iterator, l.rereadApprovals()
		}
		if !errors.Is(err, contracts.ErrReset) {
			return nil, err
		}
		timer := time.NewTimer(delay)
		select {
		case <-l.ctx.Done():
			timer.Stop()
			return nil, l.ctx.Err()
		case <-timer.C:
		}
		delay = min(delay*2, time.Second)
	}
	return nil, l.ctx.Err()
}
func (l *liveSocket) approvalEvents(after contracts.Cursor) {
	iterator, err := l.server.deps.Operations.Observe(l.ctx, l.identity.Principal, after)
	if err != nil {
		if !errors.Is(err, contracts.ErrReset) {
			l.failView(l.ctx, safeError(err).Code, "approval observation unavailable")
			return
		}
		iterator, err = l.resetApprovals()
		if err != nil {
			l.failView(l.ctx, safeError(err).Code, "approval observation unavailable")
			return
		}
	}
	for {
		batch, err := iterator.Next(l.ctx)
		if err != nil {
			if errors.Is(err, contracts.ErrReset) {
				iterator, err = l.resetApprovals()
				if err == nil {
					continue
				}
			}
			if !errors.Is(err, context.Canceled) {
				l.failView(l.ctx, safeError(err).Code, "approval observation unavailable")
			}
			return
		}
		for _, change := range batch.Changes {
			if change.Record.Collection != "system.command" {
				continue
			}
			result, err := l.server.deps.Operations.State.Read(l.ctx, l.identity.Principal.Visibility(), contracts.ReadQuery{Collection: "system.command", ID: change.Record.ID})
			if err != nil {
				l.failView(l.ctx, safeError(err).Code, "approval observation unavailable")
				return
			}
			for _, record := range result.Records {
				l.emitApprovalMarker(record)
			}
		}
	}
}
