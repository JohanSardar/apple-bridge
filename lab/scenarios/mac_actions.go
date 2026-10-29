package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func macActions(ctx context.Context, client scripted.Client, w *world.World, add func(string, bool)) {
	for _, t := range []struct{ op, args string }{
		{"mac.notify", `{"title":"bridge-lab","body":"fixture"}`},
		{"mac.open_settings", `{"pane":"notifications"}`},
		{"mac.set_wallpaper", `{"image_blob":"aW1hZ2U="}`},
	} {
		var out contracts.CommandRecord
		err := client.Call(ctx, t.op, contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(t.args), WaitMS: 15000}, &out)
		add(t.op, err == nil && out.State == contracts.CommandObserved && contracts.MacCompletionEvidence(out.Evidence, out.RequestHash))
	}
	for _, permission := range []string{"not_determined", "denied"} {
		e := w.ControlAction("/mac-notifications", map[string]any{"permission": permission})
		add("notification_permission_fixture", e == nil)
		var row contracts.CommandRecord
		e = client.Call(ctx, "mac.notify", contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{"title":"bridge-lab","body":"fixture"}`), WaitMS: 15000}, &row)
		add("notify_"+permission+"_rejected", e != nil || row.State == contracts.CommandRejected)
	}

	for _, op := range []string{"mac.lock_screen", "mac.sleep_display", "mac.restart", "mac.shutdown"} {
		var out contracts.CommandRecord
		err := client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{}`)}, &out)
		add(op+"_requires_approve", err != nil)
	}
	authority, err := trust.Open(w.Gateway.Home)
	add("approval_authority", err == nil)
	if err == nil {
		for _, level := range []trust.AgentLevel{trust.AgentFull, trust.AgentApprove} {
			_, token, e := authority.CreateAgent("Bridge Test Mac power", level, []contracts.Grant{"mac:write"})
			add("power_agent_created", e == nil)
			endpoint := w.Gateway
			endpoint.Token = token
			agent := scripted.Client{Endpoint: endpoint}
			for _, op := range []string{"mac.lock_screen", "mac.sleep_display", "mac.restart", "mac.shutdown"} {
				var row contracts.CommandRecord
				e = agent.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{}`)}, &row)
				if level == trust.AgentApprove {
					add(op+"_awaiting_approval", e == nil && row.State == contracts.CommandAwaitingApproval)
				} else {
					add(op+"_full_agent_denied", e != nil)
				}
			}
		}
	}

	var out contracts.QueryResult
	err = client.Call(ctx, "mac.network_check", contracts.Query{Args: json.RawMessage(`{}`)}, &out)
	add("network_query", err == nil)
}
