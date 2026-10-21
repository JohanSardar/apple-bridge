package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func runMessagesInteractive(ctx context.Context, w *world.World, conn *websocket.Conn, e *Evidence, add func(string, bool)) {
	submit := func(id string, extra map[string]any) contracts.CommandRecord {
		frame := map[string]any{"t": "cmd", "id": id, "op": "messages.send", "args": map[string]any{"handles": []string{"other@example.com"}, "service": "iMessage", "text": "interactive message"}}
		for k, v := range extra {
			frame[k] = v
		}
		bounded, cancel := context.WithTimeout(ctx, 20*time.Second)
		defer cancel()
		if wsjson.Write(bounded, conn, frame) != nil {
			return contracts.CommandRecord{}
		}
		for {
			var response struct {
				T       string                  `json:"t"`
				Command contracts.CommandRecord `json:"command"`
			}
			if wsjson.Read(bounded, conn, &response) != nil {
				return contracts.CommandRecord{}
			}
			if response.T == "cmd_status" && response.Command.Command.ID == id && response.Command.Disposition != "" {
				return response.Command
			}
		}
	}
	id := contracts.NewCommandID()
	started := time.Now()
	result := submit(id, nil)
	add("interactive_untagged_nonown_observed", result.State == contracts.CommandObserved && result.Command.Origin == "interactive")
	e.Latency["interactive_submit_to_observed"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	retry := submit(id, nil)
	effects, err := w.Effects()
	add("interactive_retry_never_redispatches", err == nil && effects[id] == 1 && retry.CreatedAt.Equal(result.CreatedAt))
	raw, _ := json.Marshal(map[string]any{"handles": []string{"other@example.com"}, "service": "iMessage", "text": "interactive message"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw, Origin: "interactive", WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	var blocked contracts.CommandRecord
	err = client.Call(ctx, command.Op, command, &blocked)
	effects, problem := w.Effects()
	add("lab_cannot_forge_origin", err != nil && problem == nil && effects[command.ID] == 0)
	raw, _ = json.Marshal(map[string]any{"handles": []string{"other@example.com"}, "service": "iMessage", "text": "[ab-test " + e.RunID + "] lab"})
	command.ID = contracts.NewCommandID()
	command.Args = raw
	err = client.Call(ctx, command.Op, command, &blocked)
	effects, problem = w.Effects()
	add("lab_nonown_route_blocked", err != nil && problem == nil && effects[command.ID] == 0)
	var records contracts.ReadResult
	err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.message", Limit: 100}, &records)
	found := 0
	for _, record := range records.Records {
		var data map[string]any
		json.Unmarshal(record.Data, &data)
		if data["text"] == "interactive message" && data["is_from_me"] == true {
			found++
		}
	}
	add("one_outgoing_truth_record", err == nil && found == 1)
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("agent_authority", false)
		return
	}
	for _, level := range []trust.AgentLevel{trust.AgentRead, trust.AgentApprove, trust.AgentFull} {
		_, token, problem := authority.CreateAgent("Bridge Test gate", level, []contracts.Grant{"messages:write"})
		if problem != nil {
			add("agent_created", false)
			continue
		}
		endpoint := w.Gateway
		endpoint.Token = token
		agent := scripted.Client{Endpoint: endpoint}
		command.ID = contracts.NewCommandID()
		command.Args, _ = json.Marshal(map[string]any{"handles": []string{"other@example.com"}, "service": "iMessage", "text": "agent gate"})
		result, problem := func() (contracts.CommandRecord, error) {
			var r contracts.CommandRecord
			err := agent.Call(ctx, command.Op, command, &r)
			return r, err
		}()
		effects, _ = w.Effects()
		if level == trust.AgentApprove {
			add("agent_approve_awaiting_owner", problem == nil && result.State == contracts.CommandAwaitingApproval && result.Disposition == contracts.DispositionAwaitingApproval && effects[command.ID] == 0)
			continue
		}
		if level == trust.AgentRead {
			add("agent_"+string(level)+"_blocked", problem != nil && effects[command.ID] == 0)
			continue
		}
		add("full_agent_interactive_routing", problem == nil && result.Command.Origin == "interactive" && result.State == contracts.CommandObserved && effects[command.ID] == 1)
		command.ID = contracts.NewCommandID()
		command.ExpiresAt = time.Now().Add(-time.Second)
		problem = agent.Call(ctx, command.Op, command, &blocked)
		effects, _ = w.Effects()
		add("interactive_expiry_before_dispatch", problem != nil && effects[command.ID] == 0)
		command.ExpiresAt = time.Time{}
		command.ID = contracts.NewCommandID()
		add("ambiguity_armed", w.ControlAction("/fault", map[string]any{"name": "identical-manual-send"}) == nil)
		problem = agent.Call(ctx, command.Op, command, &blocked)
		add("interactive_ambiguous_effect_indeterminate", problem == nil && blocked.State == contracts.CommandIndeterminate)
		problem = agent.Call(ctx, command.Op, command, &blocked)
		effects, _ = w.Effects()
		add("interactive_ambiguous_effect_never_redispatched", problem == nil && blocked.State == contracts.CommandIndeterminate && effects[command.ID] == 1)
		add("capability_revoked", w.ControlAction("/messages", map[string]any{"action": "capability-unproven"}) == nil)
		command.ID = contracts.NewCommandID()
		problem = agent.Call(ctx, command.Op, command, &blocked)
		effects, _ = w.Effects()
		add("interactive_capability_proof_still_required", problem != nil && effects[command.ID] == 0)
	}
}

func submitInteractiveMessage(ctx context.Context, conn *websocket.Conn, command contracts.Command) (contracts.CommandRecord, error) {
	bounded, cancel := context.WithTimeout(ctx, 25*time.Second)
	defer cancel()
	if err := wsjson.Write(bounded, conn, map[string]any{"t": "cmd", "id": command.ID, "op": command.Op, "args": command.Args}); err != nil {
		return contracts.CommandRecord{}, err
	}
	for {
		var response struct {
			T       string                  `json:"t"`
			Command contracts.CommandRecord `json:"command"`
			Error   *contracts.Error        `json:"error"`
		}
		if err := wsjson.Read(bounded, conn, &response); err != nil {
			return contracts.CommandRecord{}, err
		}
		if response.T == "cmd_status" && response.Command.Command.ID == command.ID && response.Command.Disposition != "" {
			if response.Error != nil {
				return response.Command, response.Error
			}
			return response.Command, nil
		}
	}
}
