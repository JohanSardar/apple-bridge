package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func gatewayCommands(ctx context.Context, w *world.World, e *Evidence) {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	add("dial_pairing_code_replay_rejected", w.DialPairingReplayRejected(0))
	records, err := w.Records(w.Replicas[0])
	var list string
	for _, r := range records {
		if r.Collection == "reminders.list" {
			list = r.ID
		}
	}
	if err != nil || list == "" {
		add("command_fixture", false)
		return
	}
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Replicas[0], Protocol: protocol}
		args, _ := json.Marshal(map[string]string{"list_id": list, "title": "[ab-test] gateway-dials-" + protocol})
		c := contracts.Command{ID: contracts.NewCommandID(), Op: "reminders.create", Args: args, WaitMS: 15000}
		var first, retry contracts.CommandRecord
		err := client.Call(ctx, c.Op, c, &first)
		effects, effectsErr := w.Effects()
		add("feed_command_"+protocol, err == nil && first.Disposition == contracts.DispositionLocallyObserved && effectsErr == nil && effects[c.ID] == 1)
		add("drop_feed_response_"+protocol, w.DropCommandResponse() == nil)
		lost := c
		lost.ID = contracts.NewCommandID()
		var ignored contracts.CommandRecord
		err = client.Call(ctx, lost.Op, lost, &ignored)
		add("feed_response_lost_"+protocol, errors.Is(err, contracts.ErrUnavailable))
		add("feed_reconnect_"+protocol, w.Await(15*time.Second, func() bool { status, err := w.Status(w.Replicas[0]); return err == nil && status.State == "connected" }) == nil)
		err = client.Call(ctx, lost.Op, lost, &retry)
		effects, effectsErr = w.Effects()
		add("original_id_retry_one_effect_"+protocol, err == nil && retry.Command.ID == lost.ID && retry.Disposition == contracts.DispositionLocallyObserved && effectsErr == nil && effects[lost.ID] == 1)
		var status contracts.CommandRecord
		err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: lost.ID}, &status)
		add("feed_status_"+protocol, err == nil && status.Command.ID == lost.ID && status.Disposition == retry.Disposition)
		w.Partition(0)
		add("feed_disconnect_"+protocol, w.Await(5*time.Second, func() bool {
			status, err := w.Status(w.Replicas[0])
			return err == nil && status.State == "disconnected"
		}) == nil)
		offline := c
		offline.ID = contracts.NewCommandID()
		started := time.Now()
		err = client.Call(ctx, offline.Op, offline, nil)
		effects, effectsErr = w.Effects()
		add("feed_offline_fail_fast_"+protocol, errors.Is(err, contracts.ErrUnavailable) && time.Since(started) < time.Second && effectsErr == nil && effects[offline.ID] == 0)
		w.Heal(0)
		add("feed_heal_"+protocol, w.Await(15*time.Second, func() bool { status, err := w.Status(w.Replicas[0]); return err == nil && status.State == "connected" }) == nil)
	}
}

func gatewayRevoke(ctx context.Context, w *world.World, e *Evidence) {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	before, err := w.Status(w.Replicas[0])
	add("revoke_gateway_dial_replica", err == nil && w.RevokeReplica(0) == nil)
	add("revoke_closes_feed", w.Await(2*time.Second, func() bool { s, err := w.Status(w.Replicas[0]); return err == nil && s.State == "disconnected" }) == nil)
	statuses, statusErr := w.GatewayDialStatus()
	add("gateway_reports_revoked", statusErr == nil && len(statuses) == 1 && statuses[0].State == "revoked")
	var attempts uint64
	if len(statuses) == 1 {
		attempts = statuses[0].Attempts
	}
	sleep(ctx, 6*time.Second)
	statuses, statusErr = w.GatewayDialStatus()
	add("revoked_gateway_stops_redial", statusErr == nil && len(statuses) == 1 && statuses[0].Attempts == attempts && statuses[0].State == "revoked")
	after, err := w.Status(w.Replicas[0])
	add("revoked_no_reconnect", err == nil && after.State == "disconnected" && before.Resumes == after.Resumes)
	add("unpaired_dial_certificate_rejected", w.UnpairedDialRejected(0) == nil)
}
