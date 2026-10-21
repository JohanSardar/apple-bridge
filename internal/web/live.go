package web

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
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
func (l *liveSocket) send(value any, revoke bool) bool {
	data, err := json.Marshal(value)
	if err != nil {
		return false
	}
	size := int64(len(data))
	if l.pending.Add(size) > 4*1024*1024 {
		l.pending.Add(-size)
		l.cancel()
		go l.conn.Close(websocket.StatusPolicyViolation, "overloaded")
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
		l.cancel()
		go l.conn.Close(websocket.StatusPolicyViolation, "overloaded")
		return false
	}
}
func (l *liveSocket) fail(code, message string) {
	l.send(map[string]any{"t": "error", "code": code, "message": message}, false)
}
func (l *liveSocket) write() {
	defer l.cancel()
	for {
		select {
		case <-l.ctx.Done():
			return
		case frame := <-l.queue:
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
	l := &liveSocket{server: s, conn: conn, identity: current(r), ctx: ctx, cancel: cancel, queue: make(chan liveFrame, 4096), subscriptions: map[string]context.CancelFunc{}}
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
	l.send(map[string]any{"t": "welcome", "session": l.identity.Session, "head": s.deps.Operations.Head()}, false)
	s.mu.Lock()
	s.sockets[l] = true
	s.mu.Unlock()
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
				_, _, err := s.deps.Authority.InspectDevice(l.identity.Token)
				if err != nil {
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
		var f clientFrame
		if typ != websocket.MessageText || json.Unmarshal(data, &f) != nil {
			l.fail("invalid", "invalid frame")
			continue
		}
		if _, _, err := s.deps.Authority.AuthenticateDevice(l.identity.Token); err != nil {
			l.send(map[string]any{"t": "session_revoked"}, true)
			continue
		}
		if !s.allow(l.identity.Principal.ID, false) {
			l.fail("rate_limited", "rate limited")
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
	l.send(map[string]any{"t": "cmd_status", "id": f.ID, "status": status.State, "command": status}, false)
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
				l.send(map[string]any{"t": "cmd_status", "id": f.ID, "status": next.State, "command": next}, false)
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
		return l.server.deps.Operations.Read(ctx, l.identity.Principal, "records.get", q)
	}
	if f.View == "chats" || f.View == "thread" || f.View == "collection" {
		return l.server.deps.Operations.Invoke(ctx, l.identity.Principal, "messages."+f.View, f.Args)
	}
	return nil, contracts.ErrUnsupported
}
func (l *liveSocket) subscribe(ctx context.Context, f clientFrame) {
	var recordArgs contracts.ReadQuery
	if f.View == "record" && contracts.DecodeInput(f.Args, &recordArgs) != nil {
		l.fail("invalid", "invalid record arguments")
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
			l.fail(safeError(err).Code, "view unavailable")
			return
		}
		previous = decodeViewState(data)
		if previous.Cursor.Epoch != "" {
			cursor = previous.Cursor
		}
		if !l.send(map[string]any{"t": "snap", "sub": f.ID, "data": data, "cursor": cursor}, false) {
			return
		}
		if f.View == "thread" && len(previous.Order) > 0 {
			args := map[string]any{}
			json.Unmarshal(f.Args, &args)
			delete(args, "before")
			delete(args, "after")
			args["anchor"] = previous.Order[len(previous.Order)/2]
			args["limit"] = len(previous.Order)
			f.Args, _ = json.Marshal(args)
		}
	}
	iterator, err := l.server.deps.Operations.Observe(ctx, l.identity.Principal, cursor)
	if err != nil {
		l.fail("reset", "resubscribe required")
		return
	}
	outside := map[string]string{}
	for {
		batch, err := iterator.Next(ctx)
		if err != nil {
			if ctx.Err() == nil {
				l.fail("reset", "resubscribe required")
			}
			return
		}
		cursor = contracts.Cursor{Epoch: batch.Epoch, SubscriptionRevision: batch.SubscriptionRevision, Sequence: batch.Through}
		changes := []any{}
		affected := false
		for _, change := range batch.Changes {
			if f.View == "record" {
				if change.Record.Collection == recordArgs.Collection && change.Record.ID == recordArgs.ID {
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
				l.fail("reset", "resubscribe required")
				return
			}
			next := decodeViewState(data)
			if f.View == "thread" {
				next = threadWindow(previous, next, batch.Changes, outside, f.Args)
			}
			frame["changes"] = viewChanges(previous, next)
			frame["order"] = next.Order
			frame["page"] = next.Page
			if f.View == "thread" {
				newer, older := 0, 0
				for _, side := range outside {
					if side == "newer" {
						newer++
					} else {
						older++
					}
				}
				frame["newer"] = newer
				frame["older"] = older
			}
			previous = next
			if next.Cursor.Epoch == cursor.Epoch && next.Cursor.SubscriptionRevision == cursor.SubscriptionRevision && next.Cursor.Sequence > cursor.Sequence {
				cursor = next.Cursor
				frame["cursor"] = cursor
				iterator, err = l.server.deps.Operations.Observe(ctx, l.identity.Principal, cursor)
				if err != nil {
					l.fail("reset", "resubscribe required")
					return
				}
			}
		}
		if !l.send(frame, false) {
			return
		}
	}
}

type viewState struct {
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
	} else {
		json.Unmarshal(object["items"], &rows)
	}
	for _, row := range rows {
		var identity struct {
			ID   string `json:"id"`
			Chat struct {
				ID string `json:"id"`
			} `json:"chat"`
		}
		json.Unmarshal(row, &identity)
		id := identity.ID
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
			changes = append(changes, map[string]any{"op": "upsert", "id": id, "data": row, "order": index})
		}
	}
	return changes
}
func rowSentAt(row json.RawMessage) string {
	var record struct {
		Data struct {
			SentAt string `json:"sent_at"`
		} `json:"data"`
	}
	json.Unmarshal(row, &record)
	return record.Data.SentAt
}
func threadWindow(previous, next viewState, changes []contracts.Change, outside map[string]string, args json.RawMessage) viewState {
	if len(previous.Order) == 0 {
		return next
	}
	var query struct {
		ChatID string `json:"chat_id"`
	}
	json.Unmarshal(args, &query)
	first, last := rowSentAt(previous.Rows[previous.Order[0]]), rowSentAt(previous.Rows[previous.Order[len(previous.Order)-1]])
	for _, change := range changes {
		if change.Record.Collection != "messages.message" {
			continue
		}
		if change.Op == "delete" || change.Op == "evict" || change.Record.Deleted {
			delete(outside, change.Record.ID)
			delete(previous.Rows, change.Record.ID)
			continue
		}
		var data struct {
			ChatID string `json:"chat_id"`
			SentAt string `json:"sent_at"`
		}
		json.Unmarshal(change.Record.Data, &data)
		if data.ChatID != query.ChatID {
			continue
		}
		if data.SentAt > last {
			outside[change.Record.ID] = "newer"
		} else if data.SentAt < first {
			outside[change.Record.ID] = "older"
		}
	}
	for id, row := range previous.Rows {
		if _, ok := next.Rows[id]; !ok {
			next.Rows[id] = row
		}
	}
	for id, row := range next.Rows {
		if at := rowSentAt(row); at < first || at > last {
			if _, existed := previous.Rows[id]; !existed {
				delete(next.Rows, id)
			}
		}
	}
	next.Order = next.Order[:0]
	for id := range next.Rows {
		next.Order = append(next.Order, id)
	}
	slices.SortFunc(next.Order, func(a, b string) int {
		if at, bt := rowSentAt(next.Rows[a]), rowSentAt(next.Rows[b]); at != bt {
			return strings.Compare(at, bt)
		}
		return strings.Compare(a, b)
	})
	return next
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
