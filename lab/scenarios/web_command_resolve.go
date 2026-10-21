package scenarios

import (
	"context"
	"encoding/json"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

type webResolveReply struct {
	T         string                  `json:"t"`
	ID        string                  `json:"id"`
	Command   contracts.CommandRecord `json:"command"`
	Throttled bool                    `json:"throttled"`
	Code      string                  `json:"code"`
	Error     *contracts.Error        `json:"error"`
}

func webResolve(ctx context.Context, socket *websocket.Conn, id string) (webResolveReply, error) {
	bounded, stop := context.WithTimeout(ctx, 5*time.Second)
	defer stop()
	var reply webResolveReply
	if err := wsjson.Write(bounded, socket, map[string]any{"t": "resolve", "id": id}); err != nil {
		return reply, err
	}
	for {
		if err := wsjson.Read(bounded, socket, &reply); err != nil {
			return reply, err
		}
		if reply.T == "error" || reply.T == "cmd_status" && reply.ID == id {
			return reply, nil
		}
	}
}

func runWebCommandResolve(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	var lists contracts.ReadResult
	err := w.Await(5*time.Second, func() bool {
		_, problem := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.list"}, &lists)
		return problem == nil && len(lists.Records) > 0
	})
	add("resolve_fixture_list", err == nil && len(lists.Records) > 0)
	if err != nil || len(lists.Records) == 0 {
		return
	}
	socket, err := c.socket()
	add("resolve_socket", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	raw, _ := json.Marshal(map[string]any{"list_id": lists.Records[0].ID, "title": "[ab-test " + e.RunID + "] resolve"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "reminders.create", Args: raw}
	add("resolve_ambiguous_fault", w.ControlAction("/fault", map[string]any{"name": "ambiguous-outcome"}) == nil)
	initial, err := uploadCommandReply(ctx, socket, command)
	add("resolve_initial_server_indeterminate", err == nil && initial.State == contracts.CommandIndeterminate)
	if err != nil || initial.State != contracts.CommandIndeterminate {
		return
	}
	effects, effectErr := w.Effects()
	add("resolve_initial_effect_is_one", effectErr == nil && effects[command.ID] == 1)
	for i := 0; i < 11; i++ {
		id := contracts.NewCommandID()
		unknown, problem := webResolve(ctx, socket, id)
		if i == 0 {
			add("resolve_error_identifies_requested_command", problem == nil && unknown.ID == id)
		}
		add("resolve_unknown_is_not_found_without_quota", problem == nil && unknown.T == "error" && unknown.Code == "not_found" && unknown.Command.Command.ID == "")
	}
	authority, problem := trust.Open(w.Gateway.Home)
	add("resolve_other_session_authority", problem == nil)
	if problem == nil {
		token, _, problem := authority.CreateDeviceSession("trusted")
		add("resolve_other_session", problem == nil)
		if problem == nil {
			other := &webClient{ctx: ctx, origin: c.origin, cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
			foreign, problem := other.socket()
			add("resolve_other_socket", problem == nil)
			if problem == nil {
				hidden, problem := webResolve(ctx, foreign, command.ID)
				add("resolve_foreign_owned_id_is_hidden", problem == nil && hidden.T == "error" && hidden.Code == "not_found" && hidden.Command.Command.ID == "")
				foreign.CloseNow()
			}
		}
	}
	resolved, err := webResolve(ctx, socket, command.ID)
	add("resolve_check_strengthens_original_id", err == nil && resolved.T == "cmd_status" && !resolved.Throttled && resolved.Command.Command.ID == command.ID && resolved.Command.State == contracts.CommandObserved && len(resolved.Command.ObservedRecordIDs) == 1 && resolved.Command.CommitSeq > 0)
	if err != nil || resolved.T != "cmd_status" || resolved.Command.State != contracts.CommandObserved {
		return
	}
	throttled, err := webResolve(ctx, socket, command.ID)
	add("resolve_per_command_throttle_returns_stored_status", err == nil && throttled.Throttled && throttled.Command.Cursor == resolved.Command.Cursor && throttled.Command.State == contracts.CommandObserved)
	reconnected, err := c.socket()
	add("resolve_reconnect_socket", err == nil)
	if err == nil {
		repeated, problem := webResolve(ctx, reconnected, command.ID)
		add("resolve_reconnect_cannot_bypass_throttle", problem == nil && repeated.Throttled && repeated.Command.Cursor == resolved.Command.Cursor)
		reconnected.CloseNow()
	}
	for i := 0; i < 10; i++ {
		next := contracts.Command{ID: contracts.NewCommandID(), Op: command.Op, Args: raw}
		stored, problem := uploadCommandReply(ctx, socket, next)
		add("resolve_quota_command_observed", problem == nil && stored.State == contracts.CommandObserved)
		if problem != nil || stored.State != contracts.CommandObserved {
			return
		}
		result, problem := webResolve(ctx, socket, next.ID)
		add("resolve_session_minute_limit", problem == nil && result.T == "cmd_status" && result.Throttled == (i == 9) && result.Command.Cursor == stored.Cursor)
	}
	effects, err = w.Effects()
	add("resolve_original_effect_remains_one", err == nil && effects[command.ID] == 1)
}
