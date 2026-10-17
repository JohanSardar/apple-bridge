package operations

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"time"
)

func (g *LocalGateway) executeNote(r contracts.CommandRecord, p trust.Principal, c contracts.Command, expiry, deadline time.Time, hash string) (contracts.CommandRecord, error) {
	a, err := contracts.ValidateNoteCommand(c)
	if err != nil {
		return r, err
	}
	ctx, cancel := context.WithDeadline(g.Context, deadline)
	defer cancel()
	release, err := g.lock(ctx, "object:notes:"+a.ID+":"+a.FolderID)
	if err != nil {
		return g.transition(g.Context, r, "expired", "expired", nil, nil)
	}
	defer release()
	reject := func(problem *contracts.Error) (contracts.CommandRecord, error) {
		return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, problem)
	}
	action := contracts.NativeAction{Op: c.Op, NoteArgs: &a}
	var expected json.RawMessage
	if a.ID != "" {
		result, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "notes.note", ID: a.ID})
		if err != nil || len(result.Records) != 1 {
			return reject(contracts.ErrNotFound)
		}
		current := result.Records[0]
		if current.Version != c.ExpectedVersion {
			return reject(&contracts.Error{Code: "conflict", Message: "expected_version differs from current", Current: &current})
		}
		var data struct {
			Editable string `json:"editable"`
		}
		if json.Unmarshal(current.Data, &data) != nil {
			return reject(contracts.ErrCorrupt)
		}
		if c.Op == "notes.update" && data.Editable != "plain" {
			return reject(&contracts.Error{Code: "unsupported", Message: "rich_content"})
		}
		expected = current.Data
		action.NativeKey, err = g.State.NativeKey(ctx, "notes.note", a.ID)
		if err != nil {
			return reject(contracts.ErrNotFound)
		}
	}
	if a.FolderID != "" {
		result, readErr := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "notes.folder", ID: a.FolderID})
		if readErr != nil || len(result.Records) != 1 {
			return reject(contracts.ErrNotFound)
		}
		action.FolderKey, err = g.State.NativeKey(ctx, "notes.folder", a.FolderID)
		if err != nil {
			return reject(contracts.ErrNotFound)
		}
	}
	prepared, err := g.Peer.Exchange(ctx, contracts.IPCFrame{T: "Prepare", CommandID: c.ID, Principal: p.ID, RequestHash: hash, Action: &action, ExpectedState: expected, ExpiresAt: expiry})
	if err != nil {
		return reject(contracts.ErrUnavailable)
	}
	if prepared.T == "Rejected" {
		return reject(prepared.Error)
	}
	if prepared.T != "Prepared" || prepared.Ticket == "" {
		return reject(contracts.ErrCorrupt)
	}
	evidence, _ := json.Marshal(prepared)
	r, err = g.transition(g.Context, r, "prepared", "", evidence, nil)
	if err != nil {
		return r, err
	}
	if err = g.authorize(p, c.Op); err != nil {
		return reject(contracts.ErrUnauthorized)
	}
	if !time.Now().Before(deadline) {
		return g.transition(g.Context, r, "expired", "expired", nil, nil)
	}
	r, err = g.transition(g.Context, r, "dispatched", "", nil, nil)
	if err != nil {
		return r, err
	}
	outcomeCtx, stop := context.WithTimeout(g.Context, 15*time.Second)
	defer stop()
	outcome, err := g.Peer.Exchange(outcomeCtx, contracts.IPCFrame{T: "Dispatch", CommandID: c.ID, Ticket: prepared.Ticket})
	if err != nil {
		return g.transition(g.Context, r, "indeterminate", "indeterminate", nil, nil)
	}
	return g.outcome(g.Context, r, outcome)
}
