package operations

import (
	"context"
	"encoding/json"
	"errors"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type CommandTarget interface {
	Submit(context.Context, trust.Principal, contracts.Command) (contracts.CommandRecord, error)
	Status(context.Context, trust.Principal, contracts.CommandQuery) (contracts.CommandRecord, error)
	List(context.Context, trust.Principal, contracts.CommandQuery) ([]contracts.CommandRecord, error)
}
type LocalGateway struct {
	State     *state.Store
	Authority *trust.Authority
	Peer      *applepeer.Commands
	Context   context.Context
	Audit     func(contracts.CommandRecord)
	mu        sync.Mutex
	slots     map[string]chan struct{}
}

func (g *LocalGateway) lock(ctx context.Context, key string) (func(), error) {
	g.mu.Lock()
	if g.slots == nil {
		g.slots = map[string]chan struct{}{}
	}
	ch := g.slots[key]
	if ch == nil {
		ch = make(chan struct{}, 1)
		g.slots[key] = ch
	}
	g.mu.Unlock()
	select {
	case ch <- struct{}{}:
		return func() { <-ch }, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}
func (g *LocalGateway) authorize(p trust.Principal, op string) error {
	descriptor, ok := contracts.Descriptor(op)
	if !ok {
		return contracts.ErrUnsupported
	}
	if !trust.Subset([]contracts.Grant{descriptor.Scope}, p.Grants) {
		return contracts.ErrUnauthorized
	}
	return g.Authority.Authorize(p, descriptor.Scope)
}
func (g *LocalGateway) transition(ctx context.Context, r contracts.CommandRecord, to contracts.CommandState, disposition contracts.Disposition, evidence json.RawMessage, problem *contracts.Error) (contracts.CommandRecord, error) {
	from := r.State
	r.State = to
	r.Disposition = disposition
	r.Error = problem
	if evidence != nil {
		r.Evidence = evidence
	}
	record, err := g.State.TransitionCommand(ctx, state.CommandTransition{From: from, Record: r})
	if err == nil && g.Audit != nil {
		g.Audit(record)
	}
	return record, err
}
func (g *LocalGateway) Submit(ctx context.Context, p trust.Principal, c contracts.Command) (contracts.CommandRecord, error) {
	var empty contracts.CommandRecord
	a, err := contracts.ValidateCommand(c)
	if err != nil {
		return empty, err
	}
	if err = g.authorize(p, c.Op); err != nil {
		return empty, err
	}
	hash, err := contracts.CommandHash(c)
	if err != nil {
		return empty, err
	}
	unlock, err := g.lock(ctx, "command:"+c.ID)
	if err != nil {
		return empty, err
	}
	defer func() {
		if unlock != nil {
			unlock()
		}
	}()
	if old, err := g.State.Command(ctx, c.ID); err == nil {
		if old.Principal != p.ID || old.RequestHash != hash {
			return empty, &contracts.Error{Code: "id_reused", Message: "command ID already identifies another request"}
		}
		if old.Error != nil {
			return old, old.Error
		}
		return old, nil
	} else if !errors.Is(err, contracts.ErrNotFound) {
		return empty, err
	}
	era, err := g.State.AdmissionEra(ctx)
	if err != nil {
		return empty, err
	}
	if c.AdmissionEra != "" && c.AdmissionEra != era {
		return empty, &contracts.Error{Code: "expired", Message: "admission era is lookup only"}
	}
	now := gatewayNow()
	ttl, maxTTL := contracts.CommandTTL(c.Op)
	expiry := c.ExpiresAt
	if expiry.IsZero() {
		expiry = now.Add(ttl)
	}
	if !expiry.After(now) || expiry.Sub(now) > maxTTL {
		return empty, contracts.ErrExpired
	}
	deadline := time.Now().Add(expiry.Sub(now))
	if !g.Peer.Available() {
		return empty, contracts.ErrUnavailable
	}
	r := contracts.CommandRecord{Command: c, Principal: p.ID, Source: contracts.CollectionSource(c.Op), RequestHash: hash, State: "admitted", AdmissionEra: era, CreatedAt: now, ExpiresAt: expiry}
	r, err = g.State.TransitionCommand(ctx, state.CommandTransition{Record: r})
	if err != nil {
		return r, err
	}
	if g.Audit != nil {
		g.Audit(r)
	}
	unlock()
	unlock = nil
	done := make(chan commandResult, 1)
	go func() {
		record, err := g.execute(r, p, c, a, expiry, deadline, hash)
		done <- commandResult{record, err}
	}()
	timer := time.NewTimer(time.Duration(c.WaitMS) * time.Millisecond)
	defer timer.Stop()
	select {
	case result := <-done:
		return result.Record, result.Error
	case <-timer.C:
		return g.State.Command(g.Context, c.ID)
	case <-ctx.Done():
		return g.State.Command(g.Context, c.ID)
	}
}

type commandResult struct {
	Record contracts.CommandRecord
	Error  error
}

func (g *LocalGateway) execute(r contracts.CommandRecord, p trust.Principal, c contracts.Command, a contracts.ReminderArgs, expiry, deadline time.Time, hash string) (contracts.CommandRecord, error) {
	var err error
	runctx, cancel := context.WithDeadline(g.Context, deadline)
	defer cancel()
	object := a.ID
	if object == "" {
		object = a.ListID
	}
	if object == "" {
		object = "lists"
	}
	release, err := g.lock(runctx, "object:"+object)
	if err != nil {
		return g.transition(g.Context, r, "expired", "expired", nil, nil)
	}
	defer release()
	action := contracts.NativeAction{Op: c.Op, Args: a}
	collection := "reminders.reminder"
	if a.Contact != nil {
		collection = "contacts.contact"
		action.Args = a.Contact
	}
	var current *contracts.Record
	if a.ID != "" {
		records, readErr := g.State.Read(runctx, p.Visibility(), contracts.ReadQuery{Collection: collection, ID: a.ID})
		if readErr != nil {
			problem := contracts.ErrNotFound
			r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, problem)
			return r, problem
		}
		current = &records.Records[0]
		if current.Version != c.ExpectedVersion {
			problem := &contracts.Error{Code: "conflict", Message: "expected_version differs from current", Current: current}
			r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, problem)
			return r, problem
		}
		action.NativeKey, err = g.State.NativeKey(runctx, collection, a.ID)
		var data struct {
			ListID string `json:"list_id"`
		}
		if json.Unmarshal(current.Data, &data) != nil {
			return r, contracts.ErrCorrupt
		}
		if err == nil && a.Contact == nil {
			action.ExpectedListKey, err = g.State.NativeKey(runctx, "reminders.list", data.ListID)
		}
		if a.ListID == "" {
			a.ListID = data.ListID
		}
	}
	if err == nil && a.ListID != "" {
		action.ListKey, err = g.State.NativeKey(runctx, "reminders.list", a.ListID)
	}
	if err != nil {
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
		return r, contracts.ErrNotFound
	}
	var expected json.RawMessage
	if current != nil {
		expected = current.Data
	}
	prepared, err := g.Peer.Exchange(runctx, contracts.IPCFrame{T: "Prepare", CommandID: c.ID, Principal: p.ID, RequestHash: hash, Action: &action, ExpectedState: expected, ExpiresAt: expiry})
	if err != nil {
		if !time.Now().Before(deadline) {
			return g.transition(g.Context, r, "expired", "expired", nil, nil)
		}
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrUnavailable)
		return r, contracts.ErrUnavailable
	}
	if prepared.T == "Rejected" {
		problem := g.nativeConflict(runctx, p, a.ID, action.NativeKey, collection, prepared)
		if problem == nil {
			problem = contracts.ErrInvalid
		}
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, problem)
		return r, problem
	}
	if prepared.T != "Prepared" || prepared.Ticket == "" {
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrCorrupt)
		return r, contracts.ErrCorrupt
	}
	evidence, _ := json.Marshal(prepared)
	r, err = g.transition(g.Context, r, "prepared", "", evidence, nil)
	if err != nil {
		return r, err
	}
	if !time.Now().Before(deadline) {
		return g.transition(g.Context, r, "expired", "expired", nil, nil)
	}
	if err = g.authorize(p, c.Op); err != nil {
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrUnauthorized)
		return r, err
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
	if outcome.T == "Rejected" {
		outcome.Error = g.nativeConflict(outcomeCtx, p, a.ID, action.NativeKey, collection, outcome)
	}
	return g.outcome(g.Context, r, outcome)
}

func (g *LocalGateway) outcome(ctx context.Context, r contracts.CommandRecord, f contracts.IPCFrame) (contracts.CommandRecord, error) {
	to := contracts.CommandIndeterminate
	disposition := f.Disposition
	f.Disposition = "indeterminate"
	switch disposition {
	case "locally_observed", "absent_confirmed":
		f.Disposition = disposition
		to = "observed"
	case "delivered":
		f.Disposition = disposition
		to = "delivered"
	case "failed_no_effect":
		f.Disposition = disposition
		to = "failed"
	case "delivery_failed":
		f.Disposition = disposition
		to = "delivery_failed"
	case "rejected_before_dispatch":
		f.Disposition = disposition
		to = "rejected"
	}
	if f.T == "Rejected" {
		to = "rejected"
		f.Disposition = "rejected_before_dispatch"
	}
	if to == "observed" || to == "delivered" {
		var evidence struct {
			NativeKey         string `json:"native_key"`
			Fingerprint       string `json:"fingerprint"`
			Observed          bool   `json:"observed"`
			CaptureCheckpoint string `json:"capture_checkpoint"`
		}
		collection := "reminders.reminder"
		if r.Source == contracts.Contacts {
			collection = "contacts.contact"
		}
		if r.Command.Op == "reminders.create_list" {
			collection = "reminders.list"
		}
		if json.Unmarshal(f.Evidence, &evidence) != nil || evidence.NativeKey == "" || evidence.Fingerprint != r.RequestHash || !evidence.Observed {
			to = "indeterminate"
			f.Disposition = "indeterminate"
		} else {
			sequence, err := g.Peer.CapturedSequence(r.Source, evidence.CaptureCheckpoint, collection, evidence.NativeKey)
			if err != nil || sequence.Sequence == 0 || sequence.Epoch != g.State.Head().Epoch {
				to = "indeterminate"
				f.Disposition = "indeterminate"
			} else {
				r.CommitSeq = sequence.Sequence
			}
		}
	}
	if r.State == "indeterminate" && to == "indeterminate" {
		return r, nil
	}
	result, err := g.transition(ctx, r, to, f.Disposition, f.Evidence, f.Error)
	if err == nil && result.Error != nil {
		return result, result.Error
	}
	return result, err
}
func (g *LocalGateway) Status(ctx context.Context, p trust.Principal, q contracts.CommandQuery) (contracts.CommandRecord, error) {
	r, err := g.State.Command(ctx, q.ID)
	if err != nil {
		return r, err
	}
	if r.Principal != p.ID || !p.Visibility()[r.Source] {
		return contracts.CommandRecord{}, contracts.ErrNotFound
	}
	if q.Resolve {
		era, eraErr := g.State.AdmissionEra(ctx)
		if eraErr != nil {
			return r, eraErr
		}
		if r.AdmissionEra != era {
			return r, nil
		}
	}
	if q.Resolve && r.State == "indeterminate" {
		release, err := g.lock(ctx, "command:"+q.ID)
		if err != nil {
			return r, err
		}
		defer release()
		r, err = g.State.Command(ctx, q.ID)
		if err != nil {
			return r, err
		}
		if r.State == "indeterminate" {
			f, err := g.Peer.Exchange(ctx, contracts.IPCFrame{T: "Resolve", CommandID: q.ID})
			if err != nil {
				return r, err
			}
			return g.outcome(ctx, r, f)
		}
	}
	return r, nil
}
func (g *LocalGateway) List(ctx context.Context, p trust.Principal, q contracts.CommandQuery) ([]contracts.CommandRecord, error) {
	all, err := g.State.Commands(ctx)
	out := []contracts.CommandRecord{}
	for _, r := range all {
		if r.Principal == p.ID && p.Visibility()[r.Source] {
			out = append(out, r)
		}
	}
	if q.Limit > 0 && len(out) > q.Limit {
		out = out[len(out)-q.Limit:]
	}
	return out, err
}
func (g *LocalGateway) Recover(ctx context.Context) error {
	rows, err := g.State.Commands(ctx)
	if err != nil {
		return err
	}
	for _, r := range rows {
		switch r.State {
		case "dispatched":
			_, err = g.transition(ctx, r, "indeterminate", "indeterminate", nil, nil)
		case "admitted", "prepared":
			if !gatewayNow().Before(r.ExpiresAt) {
				_, err = g.transition(ctx, r, "expired", "expired", nil, nil)
			} else {
				_, err = g.transition(ctx, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrUnavailable)
			}
		}
		if err != nil {
			return err
		}
	}
	return nil
}

func (g *LocalGateway) nativeConflict(ctx context.Context, p trust.Principal, id, nativeKey, collection string, frame contracts.IPCFrame) *contracts.Error {
	if frame.Error == nil || frame.Error.Code != "conflict" {
		return frame.Error
	}
	var observation contracts.ConflictObservation
	if json.Unmarshal(frame.Evidence, &observation) != nil || observation.NativeKey != nativeKey || nativeKey == "" {
		return contracts.ErrCorrupt
	}
	cursor, err := g.Peer.CapturedSequence(contracts.CollectionSource(collection), observation.CaptureCheckpoint, collection, nativeKey)
	if err != nil || cursor.Epoch != g.State.Head().Epoch {
		return contracts.ErrCorrupt
	}
	read, err := g.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: collection, ID: id, IncludeDeleted: true})
	if err != nil || len(read.Records) != 1 || read.Cursor.Sequence < cursor.Sequence {
		return contracts.ErrCorrupt
	}
	return &contracts.Error{Code: "conflict", Message: "native state changed before dispatch", Current: &read.Records[0]}
}
