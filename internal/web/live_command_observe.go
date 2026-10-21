package web

import (
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
)

func (l *liveSocket) observeCommand(after contracts.Cursor) (*state.ChangeIterator, error) {
	if err := commandObserveFailure("register"); err != nil {
		return nil, err
	}
	return l.server.deps.Operations.Observe(l.ctx, l.identity.Principal, after)
}
func (l *liveSocket) nextCommand(iterator *state.ChangeIterator) (contracts.Batch, error) {
	if err := commandObserveFailure("iterator"); err != nil {
		return contracts.Batch{}, err
	}
	return iterator.Next(l.ctx)
}

func (l *liveSocket) sendCommandStatus(status contracts.CommandRecord) {
	l.send(map[string]any{"t": "cmd_status", "id": status.Command.ID, "status": status.State, "command": status, "error": status.Error}, false)
}
func (l *liveSocket) localCommandStatus(id string) (contracts.CommandRecord, error) {
	result, err := l.server.deps.Operations.Read(l.ctx, l.identity.Principal, "records.get", contracts.ReadQuery{Collection: "system.command", ID: id})
	if err != nil {
		return contracts.CommandRecord{}, err
	}
	for _, record := range result.Records {
		if record.ID != id {
			continue
		}
		status, err := l.server.deps.Operations.Target.Status(l.ctx, l.identity.Principal, contracts.CommandQuery{ID: id})
		if err != nil {
			return status, err
		}
		head := l.server.deps.Operations.Head()
		if status.CommitSeq == 0 || status.Cursor.Epoch == head.Epoch && head.Sequence >= status.CommitSeq {
			return status, nil
		}
	}
	return contracts.CommandRecord{}, contracts.ErrNotFound
}
func (l *liveSocket) watchCommand(id string, iterator *state.ChangeIterator) {
	var last contracts.Cursor
	for l.ctx.Err() == nil {
		status, err := l.localCommandStatus(id)
		if err == nil && status.Cursor != last {
			l.sendCommandStatus(status)
			last = status.Cursor
			if status.Disposition != "" {
				return
			}
		}
		if iterator != nil {
			if _, err := l.nextCommand(iterator); err != nil {
				iterator = nil
			}
			continue
		}
		timer := time.NewTimer(100 * time.Millisecond)
		select {
		case <-l.ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
	}
}
