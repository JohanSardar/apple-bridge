package web

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
)

type liveFrame struct {
	Cursor *contracts.Cursor
	Data   []byte
	Revoke bool
}
type liveSocket struct {
	server        *server
	conn          *websocket.Conn
	identity      identity
	ctx           context.Context
	cancel        context.CancelFunc
	queue         chan liveFrame
	pending       atomic.Int64
	closing       atomic.Bool
	urgent        chan liveFrame
	subscriptions map[string]context.CancelFunc
	workers       sync.WaitGroup
}
type clientFrame struct {
	T               string            `json:"t"`
	V               int               `json:"v"`
	ID              string            `json:"id"`
	View            string            `json:"view"`
	Args            json.RawMessage   `json:"args"`
	Cursor          *contracts.Cursor `json:"cursor"`
	Op              string            `json:"op"`
	ExpectedVersion uint64            `json:"expected_version"`
}

func (s *server) broadcast(value any, except string) {
	s.mu.Lock()
	sockets := []*liveSocket{}
	for socket := range s.sockets {
		if socket.identity.Principal.ID != except {
			sockets = append(sockets, socket)
		}
	}
	s.mu.Unlock()
	for _, socket := range sockets {
		socket.send(value, false)
	}
}
func (l *liveSocket) overloaded() {
	if l.closing.CompareAndSwap(false, true) {
		go func() { l.conn.Close(websocket.StatusPolicyViolation, "overloaded"); l.cancel() }()
	}
}
func (l *liveSocket) send(value any, revoke bool) bool {
	if l.closing.Load() {
		return false
	}
	data, err := json.Marshal(value)
	if err != nil {
		return false
	}
	size := int64(len(data))
	if revoke {
		if !l.closing.CompareAndSwap(false, true) {
			return false
		}
		l.pending.Add(size)
		l.urgent <- liveFrame{Data: data, Revoke: true}
		time.AfterFunc(750*time.Millisecond, func() { l.cancel(); l.conn.CloseNow() })
		return true
	}
	if l.pending.Add(size) > 4*1024*1024 {
		l.pending.Add(-size)
		l.overloaded()
		return false
	}
	select {
	case l.queue <- liveFrame{Data: data, Revoke: revoke, Cursor: frameCursor(value)}:
		return true
	case <-l.ctx.Done():
		l.pending.Add(-size)
		return false
	default:
		l.pending.Add(-size)
		l.overloaded()
		return false
	}
}
func (l *liveSocket) fail(code, message string) {
	l.send(map[string]any{"t": "error", "code": code, "message": message}, false)
}
func (l *liveSocket) write() {
	defer l.cancel()
	for {
		var frame liveFrame
		select {
		case frame = <-l.urgent:
		default:
			select {
			case <-l.ctx.Done():
				return
			case frame = <-l.urgent:
			case frame = <-l.queue:
			}
		}
		{
			ctx, cancel := context.WithTimeout(l.ctx, 500*time.Millisecond)
			err := l.conn.Write(ctx, websocket.MessageText, frame.Data)
			cancel()
			l.pending.Add(-int64(len(frame.Data)))
			if err != nil {
				return
			}
			if frame.Cursor != nil && l.server.deps.FrameWritten != nil {
				l.server.deps.FrameWritten(*frame.Cursor, time.Now())
			}
			if frame.Revoke {
				l.conn.Close(websocket.StatusPolicyViolation, "session_revoked")
				return
			}
		}
	}
}
func (s *server) live(w http.ResponseWriter, r *http.Request) {
	offered := false
	for _, protocol := range strings.Split(r.Header.Get("Sec-WebSocket-Protocol"), ",") {
		if strings.TrimSpace(protocol) == "web.1" {
			offered = true
		}
	}
	if !offered {
		reject(w, 400, "protocol")
		return
	}
	conn, err := websocket.Accept(w, r, &websocket.AcceptOptions{Subprotocols: []string{"web.1"}, CompressionMode: websocket.CompressionDisabled})
	if err != nil {
		return
	}
	conn.SetReadLimit(1024 * 1024)
	ctx, cancel := context.WithCancel(r.Context())
	l := &liveSocket{server: s, conn: conn, identity: current(r), ctx: ctx, cancel: cancel, queue: make(chan liveFrame, 4096), urgent: make(chan liveFrame, 1), subscriptions: map[string]context.CancelFunc{}}
	defer func() {
		cancel()
		conn.CloseNow()
		l.workers.Wait()
		s.mu.Lock()
		delete(s.sockets, l)
		s.mu.Unlock()
	}()
	go l.write()
	helloCtx, helloCancel := context.WithTimeout(ctx, 5*time.Second)
	typ, data, err := conn.Read(helloCtx)
	helloCancel()
	var hello clientFrame
	if err != nil || typ != websocket.MessageText || json.Unmarshal(data, &hello) != nil || hello.T != "hello" || hello.V != 1 {
		conn.Close(websocket.StatusProtocolError, "hello required")
		return
	}
	head := s.deps.Operations.Head()
	l.send(map[string]any{"t": "welcome", "session": l.identity.Session, "head": head}, false)
	s.mu.Lock()
	s.sockets[l] = true
	s.mu.Unlock()
	l.workers.Add(1)
	go func() { defer l.workers.Done(); l.approvalEvents(head) }()
	l.workers.Add(1)
	go func() {
		defer l.workers.Done()
		tick := time.NewTicker(100 * time.Millisecond)
		defer tick.Stop()
		beat := time.NewTicker(15 * time.Second)
		defer beat.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-beat.C:
				l.send(map[string]any{"t": "beat", "head": s.deps.Operations.Head()}, false)
			case <-tick.C:
				principal, _, err := s.deps.Authority.InspectDevice(l.identity.Token)
				if err != nil || principal.Revision != l.identity.Principal.Revision {
					l.send(map[string]any{"t": "session_revoked"}, true)
					return
				}
			}
		}
	}()
	for {
		typ, data, err = conn.Read(ctx)
		if err != nil {
			return
		}
		if !s.allow(l.identity.Principal.ID, false) {
			l.fail("rate_limited", "rate limited")
			continue
		}
		var f clientFrame
		if typ != websocket.MessageText || json.Unmarshal(data, &f) != nil {
			l.fail("invalid", "invalid frame")
			continue
		}
		authenticate := s.deps.Authority.AuthenticateDevice
		if f.T == "ping" {
			authenticate = s.deps.Authority.InspectDevice
		}
		if principal, _, err := authenticate(l.identity.Token); err != nil || principal.Revision != l.identity.Principal.Revision {
			l.send(map[string]any{"t": "session_revoked"}, true)
			continue
		}
		switch f.T {
		case "ping":
			l.send(map[string]any{"t": "pong"}, false)
		case "unsub":
			if stop := l.subscriptions[f.ID]; stop != nil {
				stop()
				delete(l.subscriptions, f.ID)
			}
		case "sub":
			if f.ID == "" || len(f.ID) > 128 || len(l.subscriptions) >= 64 {
				l.fail("invalid", "invalid subscription")
				continue
			}
			if stop := l.subscriptions[f.ID]; stop != nil {
				stop()
			}
			subctx, stop := context.WithCancel(ctx)
			l.subscriptions[f.ID] = stop
			l.workers.Add(1)
			go func() { defer l.workers.Done(); l.subscribe(subctx, f) }()
		case "cmd":
			l.workers.Add(1)
			go func() { defer l.workers.Done(); l.command(f) }()
		default:
			l.fail("invalid", "unknown frame")
		}
	}
}
func (l *liveSocket) command(f clientFrame) {
	c := contracts.Command{ID: f.ID, Op: f.Op, Args: f.Args, ExpectedVersion: f.ExpectedVersion}
	if !contracts.ValidCommandID(c.ID) {
		l.fail("invalid", "invalid command id")
		return
	}
	d, ok := contracts.Descriptor(c.Op)
	if !ok || d.Kind != contracts.CommandOp {
		l.fail("invalid", "command operation required")
		return
	}
	if l.server.deps.Operations.Target == nil {
		l.fail("unavailable", "commands unavailable")
		return
	}
	status, err := l.server.deps.Operations.Target.Submit(l.ctx, l.identity.Principal, c)
	if err != nil && status.Command.ID == "" {
		l.send(map[string]any{"t": "ack", "id": f.ID, "status": "rejected", "error": safeError(err)}, false)
		return
	}
	l.send(map[string]any{"t": "ack", "id": f.ID, "status": status.State}, false)
	l.send(map[string]any{"t": "cmd_status", "id": f.ID, "status": status.State, "command": status, "error": status.Error}, false)
	if status.Disposition != "" {
		return
	}
	iterator, err := l.server.deps.Operations.Observe(l.ctx, l.identity.Principal, status.Cursor)
	if err != nil {
		l.fail("reset", "command observation reset")
		return
	}
	for {
		batch, err := iterator.Next(l.ctx)
		if err != nil {
			return
		}
		for _, change := range batch.Changes {
			if change.Record.Collection == "system.command" && change.Record.ID == f.ID {
				next, err := l.server.deps.Operations.Target.Status(l.ctx, l.identity.Principal, contracts.CommandQuery{ID: f.ID})
				if err != nil && next.Command.ID == "" {
					l.fail(safeError(err).Code, "command status unavailable")
					return
				}
				l.send(map[string]any{"t": "cmd_status", "id": f.ID, "status": next.State, "command": next, "error": next.Error}, false)
				if next.Disposition != "" {
					return
				}
			}
		}
	}
}
func safeError(err error) *contracts.Error {
	var typed *contracts.Error
	if errors.As(err, &typed) {
		return typed
	}
	return contracts.ErrUnavailable
}

func (l *liveSocket) view(ctx context.Context, f clientFrame) (any, error) {
	if f.View == "record" {
		var q contracts.ReadQuery
		if contracts.DecodeInput(f.Args, &q) != nil {
			return nil, contracts.ErrInvalid
		}
		result, err := l.server.deps.Operations.Read(ctx, l.identity.Principal, "records.get", q)
		if err != nil {
			return result, err
		}
		for i, record := range result.Records {
			if record.Collection == "system.command" {
				result.Records[i], err = l.commandRecord(ctx, record)
				if err != nil {
					return nil, err
				}
			}
		}
		return result, nil
	}
	if f.View == "chats" || f.View == "thread" || f.View == "collection" || f.View == "handles" {
		return l.server.deps.Operations.Invoke(ctx, l.identity.Principal, "messages."+f.View, f.Args)
	}
	return nil, contracts.ErrUnsupported
}
func (l *liveSocket) failView(ctx context.Context, code, message string) {
	if ctx.Err() == nil {
		l.fail(code, message)
	}
}
func (l *liveSocket) subscribe(ctx context.Context, f clientFrame) {
	var recordArgs contracts.ReadQuery
	if f.View == "record" && contracts.DecodeInput(f.Args, &recordArgs) != nil {
		l.failView(ctx, "invalid", "invalid record arguments")
		return
	}
	cursor := l.server.deps.Operations.Head()
	resume := false
	if f.Cursor != nil && f.View == "record" {
		cursor = *f.Cursor
		_, err := l.server.deps.Operations.Observe(ctx, l.identity.Principal, cursor)
		resume = err == nil
	}
	var previous viewState
	if !resume {
		data, err := l.view(ctx, f)
		if err != nil {
			l.failView(ctx, safeError(err).Code, "view unavailable")
			return
		}
		previous = decodeViewState(data)
		if f.View == "thread" {
			previous.Counts, err = l.windowCounts(ctx, f, previous)
			if err != nil {
				l.failView(ctx, safeError(err).Code, "window unavailable")
				return
			}
			raw, _ := json.Marshal(data)
			object := map[string]any{}
			json.Unmarshal(raw, &object)
			object["newer"] = previous.Counts.Newer
			object["older"] = previous.Counts.Older
			data = object
		}
		if previous.Cursor.Epoch != "" {
			cursor = previous.Cursor
		}
		if !l.send(map[string]any{"t": "snap", "sub": f.ID, "data": data, "cursor": cursor}, false) {
			return
		}
	}
	iterator, err := l.server.deps.Operations.Observe(ctx, l.identity.Principal, cursor)
	if err != nil {
		l.failView(ctx, "reset", "resubscribe required")
		return
	}
	for {
		batch, err := iterator.Next(ctx)
		if err != nil {
			l.failView(ctx, "reset", "resubscribe required")
			return
		}
		cursor = contracts.Cursor{Epoch: batch.Epoch, SubscriptionRevision: batch.SubscriptionRevision, Sequence: batch.Through}
		changes := []any{}
		affected := false
		for _, change := range batch.Changes {
			if f.View == "record" {
				if change.Record.Collection == recordArgs.Collection && change.Record.ID == recordArgs.ID {
					if change.Record.Collection == "system.command" {
						change.Record, err = l.commandRecord(ctx, change.Record)
						if err != nil {
							l.failView(ctx, safeError(err).Code, "command unavailable")
							return
						}
					}
					changes = append(changes, change)
				}
			} else if change.Record.Source == contracts.Messages || change.Record.Source == contracts.Contacts {
				affected = true
			}
		}
		frame := map[string]any{"t": "delta", "sub": f.ID, "changes": changes, "cursor": cursor}
		if affected {
			data, err := l.view(ctx, f)
			if err != nil {
				l.failView(ctx, "reset", "resubscribe required")
				return
			}
			next := decodeViewState(data)
			if f.View == "thread" {
				next.Counts, err = l.windowCounts(ctx, f, next)
				if err != nil {
					l.failView(ctx, "reset", "window unavailable")
					return
				}
			}
			frame["changes"] = viewChanges(previous, next)
			frame["order"] = next.Order
			frame["page"] = next.Page
			if f.View == "thread" {
				frame["newer"] = next.Counts.Newer
				frame["older"] = next.Counts.Older
				updates := frame["changes"].([]any)
				updates = append(updates, map[string]any{"op": "window", "newer": next.Counts.Newer, "older": next.Counts.Older})
				frame["changes"] = updates
			}
			previous = next
		}
		if !l.send(frame, false) {
			return
		}
	}
}

type viewState struct {
	Counts contracts.WindowCounts
	Rows   map[string]json.RawMessage
	Order  []string
	Page   map[string]any
	Cursor contracts.Cursor
}

func decodeViewState(data any) viewState {
	raw, _ := json.Marshal(data)
	var object map[string]json.RawMessage
	json.Unmarshal(raw, &object)
	state := viewState{Rows: map[string]json.RawMessage{}, Order: []string{}, Page: map[string]any{}}
	json.Unmarshal(object["cursor"], &state.Cursor)
	for _, key := range []string{"before", "after", "next", "unread_anchor", "coverage_notes"} {
		if value, ok := object[key]; ok {
			var v any
			json.Unmarshal(value, &v)
			state.Page[key] = v
		}
	}
	var rows []json.RawMessage
	if object["chats"] != nil {
		json.Unmarshal(object["chats"], &rows)
	} else if object["handles"] != nil {
		json.Unmarshal(object["handles"], &rows)
	} else {
		json.Unmarshal(object["items"], &rows)
	}
	for _, row := range rows {
		var identity struct {
			ID       string `json:"id"`
			HandleID string `json:"handle_id"`
			Chat     struct {
				ID string `json:"id"`
			} `json:"chat"`
		}
		json.Unmarshal(row, &identity)
		id := identity.ID
		if id == "" {
			id = identity.HandleID
		}
		if id == "" {
			id = identity.Chat.ID
		}
		if id != "" {
			state.Rows[id] = row
			state.Order = append(state.Order, id)
		}
	}
	return state
}
func viewChanges(previous, next viewState) []any {
	changes := []any{}
	for _, id := range previous.Order {
		if _, ok := next.Rows[id]; !ok {
			changes = append(changes, map[string]any{"op": "remove", "id": id})
		}
	}
	for index, id := range next.Order {
		row := next.Rows[id]
		if !bytes.Equal(previous.Rows[id], row) || index >= len(previous.Order) || previous.Order[index] != id {
			changes = append(changes, map[string]any{"op": "upsert", "id": id, "row": row, "order": index})
		}
	}
	return changes
}
func (l *liveSocket) windowCounts(ctx context.Context, f clientFrame, state viewState) (contracts.WindowCounts, error) {
	if len(state.Order) == 0 {
		return contracts.WindowCounts{}, nil
	}
	var args struct {
		ChatID string `json:"chat_id"`
	}
	json.Unmarshal(f.Args, &args)
	window := contracts.ThreadWindow{ChatID: args.ChatID}
	json.Unmarshal(state.Rows[state.Order[0]], &window.First)
	json.Unmarshal(state.Rows[state.Order[len(state.Order)-1]], &window.Last)
	return l.server.deps.Operations.ThreadWindowCounts(ctx, l.identity.Principal, window)
}
func frameCursor(value any) *contracts.Cursor {
	m, ok := value.(map[string]any)
	if !ok {
		return nil
	}
	cursor, ok := m["cursor"].(contracts.Cursor)
	if !ok {
		return nil
	}
	return &cursor
}

func (l *liveSocket) commandRecord(ctx context.Context, record contracts.Record) (contracts.Record, error) {
	if l.server.deps.Operations.Target == nil {
		return record, contracts.ErrUnavailable
	}
	status, err := l.server.deps.Operations.Target.Status(ctx, l.identity.Principal, contracts.CommandQuery{ID: record.ID})
	if err != nil && status.Command.ID == "" {
		return record, err
	}
	record.Data, err = json.Marshal(status)
	return record, err
}

func (l *liveSocket) approvalEvents(after contracts.Cursor) {
	iterator, err := l.server.deps.Operations.Observe(l.ctx, l.identity.Principal, after)
	if err != nil {
		return
	}
	for {
		batch, err := iterator.Next(l.ctx)
		if err != nil {
			return
		}
		for _, change := range batch.Changes {
			if change.Record.Collection != "system.command" {
				continue
			}
			var value struct {
				Principal string `json:"principal"`
			}
			if json.Unmarshal(change.Record.Data, &value) != nil || !trust.IsAgentPrincipalID(value.Principal) {
				continue
			}
			command, err := l.server.deps.Operations.State.Command(l.ctx, change.Record.ID)
			if err != nil || len(command.History) == 0 || command.History[0].State != contracts.CommandAwaitingApproval {
				continue
			}
			l.send(map[string]any{"t": "approval", "command": map[string]any{"command_id": change.Record.ID}}, false)
		}
	}
}
