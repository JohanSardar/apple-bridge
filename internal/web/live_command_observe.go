package web

import (
	"errors"
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
		commandStatusLookup(id)
		status, err := l.server.deps.Operations.Target.Status(l.ctx, l.identity.Principal, contracts.CommandQuery{ID: id})
		head := l.server.deps.Operations.Head()
		if err != nil && !(errors.Is(err, contracts.ErrReset) && status.Command.ID == id && head.Epoch > status.Cursor.Epoch) {
			return status, err
		}
		return status, nil
	}
	return contracts.CommandRecord{}, contracts.ErrNotFound
}
func commandStatusVisible(status contracts.CommandRecord, head contracts.Cursor) bool {
	return status.CommitSeq == 0 || head.Epoch > status.Cursor.Epoch || head.Epoch == status.Cursor.Epoch && head.Sequence >= status.CommitSeq
}
func (l *liveSocket) watchCommand(id string, iterator *state.ChangeIterator, status contracts.CommandRecord, last contracts.Cursor) {
	lookup := true
	delay := 100 * time.Millisecond
	for l.ctx.Err() == nil {
		if lookup {
			next, err := l.localCommandStatus(id)
			if err == nil {
				status = next
			} else if !errors.Is(err, contracts.ErrNotFound) {
				iterator = nil
			}
		}
		if status.Command.ID == id && status.Cursor != last && commandStatusVisible(status, l.server.deps.Operations.Head()) && (status.Disposition == "" || !commandVisibilityHeld(id)) {
			l.sendCommandStatus(status)
			last = status.Cursor
			if status.Disposition != "" {
				return
			}
		}
		if iterator != nil {
			batch, err := l.nextCommand(iterator)
			lookup = err != nil
			if err != nil {
				iterator = nil
				continue
			}
			for _, change := range batch.Changes {
				if change.Record.Collection == "system.command" && change.Record.ID == id {
					lookup = true
					break
				}
			}
			continue
		}
		timer := time.NewTimer(delay)
		select {
		case <-l.ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
		lookup = true
		delay = min(delay*2, time.Second)
	}
}
