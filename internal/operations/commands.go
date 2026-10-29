package operations

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"strings"
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
	QueryAudit    func(QueryAudit)
	queryRates    map[string]queryWindow
	uploadMu      sync.Mutex
	uploadPending map[string]uploadBinding

	Reauthorize func(context.Context, string, string, string) (trust.Principal, error)
	Uploads     *blobs.Store
	State       *state.Store
	Authority   *trust.Authority
	Peer        *applepeer.Commands
	Context     context.Context
	Audit       func(contracts.CommandRecord)
	mu          sync.Mutex
	slots       map[string]chan struct{}
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
func (g *LocalGateway) Submit(ctx context.Context, p trust.Principal, c contracts.Command) (result contracts.CommandRecord, resultErr error) {
	defer func() {
		if p.IsAgent() {
			r := result
			if r.Command.ID == "" {
				r = contracts.CommandRecord{Command: contracts.RedactMacCommand(c), Principal: p.ID, State: "rejected"}
			}
			if r.Cursor.Sequence == 0 {
				if err := g.recordAgentCommand(r); err != nil && resultErr == nil {
					resultErr = err
				}
			}
		}
	}()
	var empty contracts.CommandRecord
	if p.Kind == "agent" {
		current, err := g.Authority.Principal(p.ID)
		if err != nil {
			return empty, err
		}
		current.Grants = trust.EffectiveGrants(current.Grants, p.Grants)
		p = current
	}
	if p.IsAgent() && p.Level == trust.AgentRead {
		return empty, contracts.ErrUnauthorized
	}
	c.Origin = ""
	if p.Kind == "device" || p.IsAgent() && (p.Level == trust.AgentFull || p.Level == trust.AgentApprove) || p.Kind == "delegated" && strings.Contains(p.ID, "/device:") {
		c.Origin = "interactive"
	}
	a, err := contracts.ValidateCommand(c)
	if err != nil {
		return empty, err
	}
	if err = g.authorize(p, c.Op); err != nil {
		return empty, err
	}
	if c.Op == "mac.clipboard_write" && p.IsAgent() && p.Level == trust.AgentApprove {
		return empty, contracts.ErrUnsupported
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
		if old.State == contracts.CommandAwaitingApproval {
			old, err = g.expireApproval(ctx, old)
			if err != nil {
				return old, err
			}
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
	refs, releaseUploads, err := g.borrowUploads(ctx, p.ID, c)
	if err != nil {
		return empty, err
	}
	defer func() {
		if releaseUploads != nil {
			releaseUploads()
		}
	}()
	if p.IsAgent() && p.Level == trust.AgentApprove {
		r := contracts.CommandRecord{UploadReferences: refs, Command: contracts.RedactMacCommand(c), Principal: p.ID, Source: contracts.CollectionSource(c.Op), RequestHash: hash, State: contracts.CommandAwaitingApproval, Disposition: contracts.DispositionAwaitingApproval, AdmissionEra: era, CreatedAt: now, ExpiresAt: now.Add(24 * time.Hour)}
		return g.State.TransitionCommand(ctx, state.CommandTransition{Record: r})
	}
	if !g.Peer.Available() {
		return empty, contracts.ErrUnavailable
	}
	r := contracts.CommandRecord{UploadReferences: refs, Command: contracts.RedactMacCommand(c), Principal: p.ID, Source: contracts.CollectionSource(c.Op), RequestHash: hash, State: "admitted", AdmissionEra: era, CreatedAt: now, ExpiresAt: expiry}
	r, err = g.State.TransitionCommand(ctx, state.CommandTransition{Record: r})
	if err != nil {
		return r, err
	}
	releaseUploads()
	releaseUploads = nil
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
	if strings.HasPrefix(c.Op, "notes.") {
		return g.executeNote(r, p, c, expiry, deadline, hash)
	}
	var err error
	runctx, cancel := context.WithDeadline(g.Context, deadline)
	defer cancel()
	object := a.ID
	if r.Source == contracts.Calendar {
		object = g.calendarSeriesKey(runctx, p, a.ID, a.ListID)
	}
	if object == "" {
		object = a.ListID
	}
	if object == "" {
		object = "lists"
	}
	if strings.HasPrefix(c.Op, "files.") {
		object = "files"
	}
	release, err := g.lock(runctx, "object:"+object)
	if err != nil {
		return g.transition(g.Context, r, "expired", "expired", nil, nil)
	}
	defer release()
	action := contracts.NativeAction{Op: c.Op, Args: a, Origin: c.Origin}
	if strings.HasPrefix(c.Op, "music.") {
		args, problem := g.musicArgs(runctx, p, c)
		if problem != nil {
			return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, problem)
		}
		action.Args = args
	}
	if strings.HasPrefix(c.Op, "photos.") || strings.HasPrefix(c.Op, "mail.") {
		action.Args = c.Args
	}
	if contracts.PlaybackOperation(c.Op) {
		var args contracts.PlaybackArgs
		if json.Unmarshal(c.Args, &args) != nil {
			return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrInvalid)
		}
		action.Args = args
	}
	if strings.HasPrefix(c.Op, "files.") {
		var args contracts.FileArgs
		if json.Unmarshal(c.Args, &args) != nil {
			return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrInvalid)
		}
		action.Args = args
	}
	if strings.HasPrefix(c.Op, "safari.") {
		var args contracts.SafariArgs
		if json.Unmarshal(c.Args, &args) != nil {
			return r, contracts.ErrInvalid
		}
		action.Args = args
	}
	if contracts.CollectionSource(c.Op) == "mac" {
		var args contracts.MacArgs
		if json.Unmarshal(c.Args, &args) != nil {
			return r, contracts.ErrInvalid
		}
		action.Args = args
	}
	if c.Op == "maps.open" {
		var args contracts.MapsOpenArgs
		json.Unmarshal(c.Args, &args)
		action.Args = args
	}
	if c.Op == "shortcuts.run" {
		var args contracts.ShortcutRunArgs
		if json.Unmarshal(c.Args, &args) != nil {
			return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrInvalid)
		}
		action.Args = args
	}
	collection := "reminders.reminder"
	if strings.HasPrefix(c.Op, "mail.") {
		collection = "mail.message"
	}
	if strings.HasPrefix(c.Op, "photos.") {
		collection = "photos.asset"
		if c.Op == "photos.create_album" {
			collection = "photos.album"
		}
	}
	if c.Op == "reminders.rename_list" || c.Op == "reminders.delete_list" {
		collection = "reminders.list"
	}
	containerCollection, containerField := "reminders.list", "list_id"
	if r.Source == contracts.Calendar {
		ca, _ := contracts.ValidateCalendar(c)
		action.CalendarArgs = &ca
		collection, containerCollection, containerField = "calendar.event", "calendar.calendar", "calendar_id"
	}
	if a.Contact != nil {
		collection = "contacts.contact"
		if contracts.ContactGroupOperation(c.Op) {
			collection = "contacts.group"
		}
		action.Args = a.Contact
		var group contracts.ContactGroupArgs
		if contracts.ContactGroupOperation(c.Op) && json.Unmarshal(a.Contact, &group) == nil && group.ContactID != "" {
			visible, problem := g.State.Read(runctx, p.Visibility(), contracts.ReadQuery{Collection: "contacts.contact", ID: group.ContactID})
			if problem != nil || len(visible.Records) != 1 {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
			}
			action.ListKey, err = g.State.NativeKey(runctx, "contacts.contact", group.ContactID)
			if err != nil {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
			}
		}
	}
	var current *contracts.Record
	if strings.HasPrefix(c.Op, "messages.") {
		m, _ := contracts.ValidateMessagesCommand(c)
		nativeArgs := contracts.NativeMessagesArgs{MessagesArgs: m}
		nativeArgs.MessagesArgs.Files = nil
		if m.ResendOf != "" {
			previous, problem := g.State.Read(runctx, p.Visibility(), contracts.ReadQuery{Collection: "messages.message", ID: m.ResendOf})
			if problem != nil || len(previous.Records) != 1 {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
			}
			var failed struct {
				FromMe bool   `json:"is_from_me"`
				Error  int    `json:"error_code"`
				ChatID string `json:"chat_id"`
			}
			if json.Unmarshal(previous.Records[0].Data, &failed) != nil || !failed.FromMe || failed.Error == 0 || failed.ChatID != m.ChatID {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrConflict)
			}
			nativeArgs.ResendOf, err = g.State.NativeKey(runctx, "messages.message", m.ResendOf)
			if err != nil {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
			}
		}
		for i := range m.Files {
			file := &m.Files[i]
			if g.Uploads == nil {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrUnavailable)
			}
			if file.Data != "" {
				data, _ := base64.StdEncoding.DecodeString(file.Data)
				file.UploadID, err = g.Uploads.StageMessageUpload(runctx, p.ID, bytes.NewReader(data))
			}
			if err == nil {
				var path, digest string
				if ref, ok := r.UploadReferences[file.UploadID]; ok {
					var cleanup func()
					path, cleanup, err = g.Uploads.CommandUpload(runctx, c.ID, file.Name, ref)
					if cleanup != nil {
						defer cleanup()
					}
					digest = ref.Digest
				} else {
					path, digest, err = g.Uploads.ResolveMessageUpload(runctx, p.ID, file.UploadID)
				}
				nativeArgs.Files = append(nativeArgs.Files, contracts.NativeMessageFile{Name: file.Name, Path: path, Digest: digest})
			}
			if err != nil {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
			}
			defer g.Uploads.ReleaseMessageUpload(g.Context, p.ID, file.UploadID)
			file.Data = ""
		}
		action.Messages = &nativeArgs
		if m.ID != "" {
			collection = contracts.MessagesTargetCollection(c.Op)
			result, readErr := g.State.Read(runctx, p.Visibility(), contracts.ReadQuery{Collection: collection, ID: m.ID})
			if readErr != nil || len(result.Records) != 1 {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
			}
			current = &result.Records[0]
			if c.ExpectedVersion != 0 && c.ExpectedVersion != current.Version {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, &contracts.Error{Code: "conflict", Message: "expected_version differs from current", Current: current})
			}
			action.NativeKey, err = g.State.NativeKey(runctx, collection, m.ID)
		}
		if err == nil && m.ChatID != "" {
			action.ListKey, err = g.State.NativeKey(runctx, "messages.chat", m.ChatID)
		}
	}
	if !strings.HasPrefix(c.Op, "messages.") && a.ID != "" {
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
		var data map[string]json.RawMessage
		if json.Unmarshal(current.Data, &data) != nil {
			return r, contracts.ErrCorrupt
		}
		var containerID string
		json.Unmarshal(data[containerField], &containerID)
		if err == nil && a.Contact == nil && collection != "reminders.list" && collection != "mail.message" {
			action.ExpectedListKey, err = g.State.NativeKey(runctx, containerCollection, containerID)
		}
		if a.ListID == "" {
			a.ListID = containerID
		}
	}
	if err == nil && a.ListID != "" {
		action.ListKey, err = g.State.NativeKey(runctx, containerCollection, a.ListID)
	}
	if err != nil {
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrNotFound)
		return r, contracts.ErrNotFound
	}
	var expected json.RawMessage
	if current != nil {
		expected = current.Data
		if contracts.ContactGroupOperation(c.Op) {
			expected, err = g.contactGroupExpected(runctx, current.Data)
			if err != nil {
				return g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrUnavailable)
			}
		}
	}
	prepared, err := g.Peer.Exchange(runctx, contracts.IPCFrame{T: "Prepare", Origin: c.Origin, CommandID: c.ID, Principal: p.ID, RequestHash: hash, Action: &action, ExpectedState: expected, ExpiresAt: expiry})
	if err != nil {
		if !time.Now().Before(deadline) {
			return g.transition(g.Context, r, "expired", "expired", nil, nil)
		}
		r, _ = g.transition(g.Context, r, "rejected", "rejected_before_dispatch", nil, contracts.ErrUnavailable)
		return r, contracts.ErrUnavailable
	}
	if prepared.T == "Rejected" {
		problem := prepared.Error
		if r.Source != contracts.Messages && r.Source != "files" {
			problem = g.nativeConflict(runctx, p, a.ID, action.NativeKey, collection, prepared)
		}
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
	if r.Source == "music" {
		evidence, _ = json.Marshal(map[string]string{"fingerprint": hash})
	}
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
	outcomeTimeout := 15 * time.Second
	if c.Op == "shortcuts.run" {
		var args contracts.ShortcutRunArgs
		if json.Unmarshal(c.Args, &args) == nil {
			timeoutMS := args.TimeoutMS
			if timeoutMS == 0 {
				timeoutMS = 30000
			}
			if timeoutMS > 0 && timeoutMS <= 300000 {
				outcomeTimeout = time.Duration(timeoutMS)*time.Millisecond + 5*time.Second
			}
		}
	}
	outcomeCtx, stop := context.WithTimeout(g.Context, outcomeTimeout)
	defer stop()
	outcome, err := g.Peer.Exchange(outcomeCtx, contracts.IPCFrame{T: "Dispatch", Origin: c.Origin, CommandID: c.ID, Ticket: prepared.Ticket})
	if err != nil {
		return g.transition(g.Context, r, "indeterminate", "indeterminate", nil, nil)
	}
	if outcome.T == "Rejected" {
		if r.Source != contracts.Messages {
			outcome.Error = g.nativeConflict(outcomeCtx, p, a.ID, action.NativeKey, collection, outcome)
		}
	}
	return g.outcome(g.Context, r, outcome)
}

func (g *LocalGateway) outcome(ctx context.Context, r contracts.CommandRecord, f contracts.IPCFrame) (contracts.CommandRecord, error) {
	f = g.awaitMessageCapture(ctx, r, f)
	prior := r
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
	if r.Command.Op == "shortcuts.run" && to == "observed" && !contracts.ShortcutCompletionEvidence(f.Evidence, r.RequestHash) {
		to = "indeterminate"
		f.Disposition = "indeterminate"
	}
	if r.Command.Op == "maps.open" && to == "observed" && !contracts.MapsOpenEvidence(f.Evidence, r.RequestHash) {
		to = "indeterminate"
		f.Disposition = "indeterminate"
	}
	if (r.Source == "mac" || r.Source == contracts.Safari) && to == "observed" && !contracts.MacCompletionEvidence(f.Evidence, r.RequestHash) {
		to = "indeterminate"
		f.Disposition = "indeterminate"
	}
	if contracts.PlaybackOperation(r.Command.Op) && (to == "observed" || to == "delivered" || to == "delivery_failed") && (to != "observed" || !contracts.PlaybackEvidence(f.Evidence, r.RequestHash)) {
		to = "indeterminate"
		f.Disposition = "indeterminate"
	}
	if strings.HasPrefix(r.Command.Op, "files.") && to == "observed" && !contracts.FileCompletionEvidence(r.Command, f.Evidence, r.RequestHash) {
		to = "indeterminate"
		f.Disposition = "indeterminate"
	}
	if contracts.MusicPlayerOperation(r.Command.Op) && to == "observed" && !contracts.MusicCompletionEvidence(f.Evidence, r.RequestHash) {
		to = "indeterminate"
		f.Disposition = "indeterminate"
	}
	if r.Command.Op != "maps.open" && r.Source != "mac" && r.Source != contracts.Safari && r.Command.Op != "shortcuts.run" && !contracts.MusicPlayerOperation(r.Command.Op) && !contracts.PlaybackOperation(r.Command.Op) && (to == "observed" || to == "delivered" || to == "delivery_failed") {
		var evidence struct {
			NativeKey         string `json:"native_key"`
			Fingerprint       string `json:"fingerprint"`
			Observed          bool   `json:"observed"`
			CaptureCheckpoint string `json:"capture_checkpoint"`
			Ambiguous         bool   `json:"ambiguous"`
			CandidateCount    int    `json:"candidate_count"`
		}
		collection := "reminders.reminder"
		if r.Source == "music" {
			collection = "music.library_item"
			if r.Command.Op == "music.add_to_playlist" {
				collection = "music.playlist"
			}
		}

		if r.Source == "photos" {
			collection = "photos.asset"
			if r.Command.Op == "photos.create_album" {
				collection = "photos.album"
			}
		}
		if r.Source == "files" {
			collection = "files.item"
		}
		if r.Source == contracts.Contacts {
			collection = "contacts.contact"
			if contracts.ContactGroupOperation(r.Command.Op) {
				collection = "contacts.group"
			}
		}
		if r.Source == contracts.Calendar {
			collection = "calendar.event"
		}
		if r.Command.Op == "calendar.create_calendar" {
			collection = "calendar.calendar"
		}
		if r.Source == contracts.Notes {
			collection = "notes.note"
			if r.Command.Op == "notes.create_folder" || r.Command.Op == "notes.rename_folder" {
				collection = "notes.folder"
			}
		}
		if r.Source == contracts.Messages {
			collection = contracts.MessagesTargetCollection(r.Command.Op)
			if r.Command.Op == "messages.react" {
				collection = "messages.reaction"
			}
		}
		if r.Command.Op == "reminders.create_list" || r.Command.Op == "reminders.rename_list" || r.Command.Op == "reminders.delete_list" {
			collection = "reminders.list"
		}
		if json.Unmarshal(f.Evidence, &evidence) != nil || evidence.Ambiguous || evidence.CandidateCount > 1 || evidence.NativeKey == "" || evidence.Fingerprint != r.RequestHash || !evidence.Observed {
			if to != "delivery_failed" {
				to = "indeterminate"
				f.Disposition = "indeterminate"
			}
		} else {
			sequence, err := g.Peer.CapturedSequence(r.Source, evidence.CaptureCheckpoint, collection, evidence.NativeKey)
			if err != nil || sequence.Sequence == 0 || sequence.Epoch != g.State.Head().Epoch {
				if to != "delivery_failed" {
					to = "indeterminate"
					f.Disposition = "indeterminate"
				}
			} else {
				r.CommitSeq = sequence.Sequence
				if id, err := g.State.ObservedRecordID(ctx, collection, evidence.NativeKey, sequence); err == nil {
					r.ObservedRecordIDs = []string{id}
				} else if !errors.Is(err, contracts.ErrNotFound) {
					var details map[string]any
					json.Unmarshal(f.Evidence, &details)
					problem := contracts.ErrUnavailable
					var typed *contracts.Error
					if errors.As(err, &typed) {
						problem = typed
					}
					details["record_id_resolution_error"] = problem.Code
					f.Evidence, _ = json.Marshal(details)
				}
			}
		}
	}
	if r.Source == "music" {
		f.Evidence = musicPublicEvidence(f.Evidence)
	}
	if g.messageParts(ctx, &r, &f) {
		if r.CommitSeq < prior.CommitSeq {
			r.CommitSeq = prior.CommitSeq
		}
		to, f.Disposition = aggregateMessageParts(r.Parts)
		if r.State == to {
			if sameMessageParts(prior.Parts, r.Parts) {
				return prior, nil
			}
			r.Disposition = f.Disposition
			r.Evidence = f.Evidence
			result, err := g.State.UpdateCommandParts(ctx, r)
			if err == nil && g.Audit != nil {
				g.Audit(result)
			}
			return result, err
		}
	}
	if r.State == "observed" && (to == "observed" || to == "indeterminate") {
		return r, nil
	}
	if r.State == "indeterminate" && to == "indeterminate" {
		return r, nil
	}
	if r.State == contracts.CommandDispatched && to == contracts.CommandDelivered {
		var err error
		r, err = g.transition(ctx, r, contracts.CommandObserved, contracts.DispositionLocallyObserved, f.Evidence, nil)
		if err != nil {
			return r, err
		}
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
	if r.State == contracts.CommandAwaitingApproval {
		r, err = g.expireApproval(ctx, r)
		if err != nil {
			return r, err
		}
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
	if q.Resolve && (r.State == "indeterminate" || resolvableMessageParts(r)) {
		release, err := g.lock(ctx, "command:"+q.ID)
		if err != nil {
			return r, err
		}
		defer release()
		r, err = g.State.Command(ctx, q.ID)
		if err != nil {
			return r, err
		}
		if r.State == "indeterminate" || resolvableMessageParts(r) {
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
			if r.State == contracts.CommandAwaitingApproval {
				r, err = g.expireApproval(ctx, r)
				if err != nil {
					return nil, err
				}
			}
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
		case "awaiting_approval":
			_, err = g.expireApproval(ctx, r)
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
