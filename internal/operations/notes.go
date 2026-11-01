package operations

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
	"time"
)

func (g *LocalGateway) executeNote(r contracts.CommandRecord, p trust.Principal, c contracts.Command, expiry, deadline time.Time, hash string) (contracts.CommandRecord, error) {
	a, err := contracts.ValidateNoteCommand(c)
	if err != nil {
		return r, err
	}
	ctx, cancel := context.WithDeadline(g.Context, deadline)
	defer cancel()
	objectKey := a.ID
	if objectKey == "" {
		objectKey = ":" + a.FolderID
	}
	release, err := g.lock(ctx, "object:notes:"+objectKey)
	if err != nil {
		return g.transition(g.Context, r, "expired", "expired", nil, nil)
	}
	defer release()
	reject := func(problem *contracts.Error) (contracts.CommandRecord, error) {
		return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, noteCommandFailure(problem))
	}
	action := contracts.NativeAction{Op: c.Op, NoteArgs: &a, Origin: c.Origin}
	var expected json.RawMessage
	collection := "notes.note"
	if c.Op == "notes.rename_folder" {
		collection = "notes.folder"
	}
	if c.Op == "notes.rename_folder" {
		if problem := g.writableNoteFolder(ctx, p, a.ID); problem != nil {
			return reject(problem)
		}
	}
	if a.ID != "" {
		result, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: collection, ID: a.ID})
		if err != nil || len(result.Records) != 1 {
			return reject(contracts.ErrNotFound)
		}
		current := result.Records[0]
		var data struct {
			Title           string `json:"title"`
			Editable        string `json:"editable"`
			ContentVersion  uint64 `json:"content_version"`
			FolderID        string `json:"folder_id"`
			Locked          bool   `json:"locked"`
			RecentlyDeleted bool   `json:"recently_deleted"`
		}
		if json.Unmarshal(current.Data, &data) != nil {
			return reject(contracts.ErrCorrupt)
		}
		if c.Op == "notes.rename_folder" && !strings.HasPrefix(data.Title, "Bridge Test [ab-test") {
			return reject(contracts.ErrUnsupported)
		}
		contentVersion := data.ContentVersion
		if contentVersion == 0 || contentVersion > current.Version {
			contentVersion = current.Version
		}
		if c.ExpectedVersion < contentVersion || c.ExpectedVersion > current.Version {
			return reject(&contracts.Error{Code: "conflict", Message: "expected_version differs from current content", Current: &current})
		}
		if c.Op == "notes.update" && (a.HTML == nil && data.Editable != "plain" || data.Locked) {
			return reject(&contracts.Error{Code: "unsupported", Message: "rich_content"})
		}
		if c.Op == "notes.update" && a.HTML != nil && !contracts.NoteHTMLCompatible(current.Data, *a.HTML) {
			return reject(contracts.ErrUnsupported)
		}
		if data.RecentlyDeleted {
			return reject(contracts.ErrUnsupported)
		}
		if data.FolderID != "" {
			if err := g.writableNoteFolder(ctx, p, data.FolderID); err != nil {
				return reject(err)
			}
		}
		expected = current.Data
		action.NativeKey, err = g.State.NativeKey(ctx, collection, a.ID)
		if err != nil {
			return reject(contracts.ErrNotFound)
		}
	}
	if a.FolderID != "" {
		if err := g.writableNoteFolder(ctx, p, a.FolderID); err != nil {
			return reject(err)
		}
		result, readErr := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "notes.folder", ID: a.FolderID})
		if readErr != nil || len(result.Records) != 1 {
			return reject(contracts.ErrNotFound)
		}
		action.FolderKey, err = g.State.NativeKey(ctx, "notes.folder", a.FolderID)
		if err != nil {
			return reject(contracts.ErrNotFound)
		}
	}
	prepared, err := g.Peer.Exchange(ctx, contracts.IPCFrame{T: "Prepare", Origin: c.Origin, CommandID: c.ID, Principal: p.ID, RequestHash: hash, Action: &action, ExpectedState: expected, ExpiresAt: expiry})
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
	outcome, err := g.Peer.Exchange(outcomeCtx, contracts.IPCFrame{T: "Dispatch", Origin: c.Origin, CommandID: c.ID, Ticket: prepared.Ticket})
	if err != nil {
		return g.transition(g.Context, r, "indeterminate", "indeterminate", nil, nil)
	}
	outcome.Error = noteCommandFailure(outcome.Error)
	return g.outcome(g.Context, r, outcome)
}

func noteCommandFailure(problem *contracts.Error) *contracts.Error {
	if problem == nil {
		return nil
	}
	result := *problem
	switch problem.Code {
	case "unsupported":
		result.Message = "Notes can't apply this change. Use a writable plain-text note, or preserve the existing formatting in an HTML update. Edit locked or unsupported content in Notes."
	case "conflict":
		result.Message = "This note or folder changed. Refresh it and try again."
	case "unavailable":
		result.Message = "Notes didn't respond. Open Notes, check its Automation access, and try again."
	case "capability_unproven":
		result.Message = "Notes couldn't confirm this change. Check the result in Notes before trying again."
	case "not_found":
		result.Message = "This note or folder is no longer available. Refresh Notes and try again."
	case "unauthorized":
		result.Message = "This connection can't change Notes. Reconnect with Notes write access."
	case "invalid":
		result.Message = "The Notes request is incomplete or invalid. Check the note or folder details and try again."
	}
	return &result
}

func (g *LocalGateway) writableNoteFolder(ctx context.Context, p trust.Principal, id string) *contracts.Error {
	seen := map[string]bool{}
	for id != "" {
		if seen[id] {
			return contracts.ErrCorrupt
		}
		seen[id] = true
		result, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "notes.folder", ID: id})
		if err != nil || len(result.Records) != 1 {
			return contracts.ErrNotFound
		}
		var data struct {
			ParentID        string `json:"parent_id"`
			RecentlyDeleted bool   `json:"recently_deleted"`
			ReadOnly        bool   `json:"read_only"`
		}
		if json.Unmarshal(result.Records[0].Data, &data) != nil {
			return contracts.ErrCorrupt
		}
		if data.RecentlyDeleted || data.ReadOnly {
			return contracts.ErrUnsupported
		}
		id = data.ParentID
	}
	return nil
}
