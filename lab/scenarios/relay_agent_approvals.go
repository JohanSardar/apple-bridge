package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayAgentApprovalBoundaries(ctx context.Context, w *world.World, owner *webClient, authority *trust.Authority, create func(trust.AgentLevel) (trust.Agent, string), call func(string, contracts.Command) (int, contracts.CommandRecord), command func() contracts.Command, add func(string, bool)) {
	revoked, token := create(trust.AgentApprove)
	c := command()
	code, row := call(token, c)
	add("revocation_fixture_pending", code == 200 && row.State == contracts.CommandAwaitingApproval)
	err := authority.Revoke(revoked.ID)
	add("replica_agent_revoked_locally", err == nil)
	response, raw, err := owner.request("POST", "approvals/"+c.ID, map[string]string{"decision": "approve"})
	json.Unmarshal(raw, &row)
	effects, _ := w.Effects()
	add("fresh_replica_revocation_rejects_pending_approval", err == nil && response.StatusCode == 200 && row.State == contracts.CommandRejected && effects[c.ID] == 0)
	_, token = create(trust.AgentApprove)
	offline := command()
	code, row = call(token, offline)
	add("offline_fixture_pending", code == 200 && row.State == contracts.CommandAwaitingApproval)
	w.StopReplica(0)
	response, _, err = owner.request("POST", "approvals/"+offline.ID, map[string]string{"decision": "approve"})
	effects, _ = w.Effects()
	add("offline_replica_keeps_approval_pending_without_effect", err == nil && response.StatusCode == 409 && pendingReplicaApproval(owner, offline.ID) && effects[offline.ID] == 0)
	err = w.RestartReplica(0)
	add("replica_reconnects_for_fresh_approval", err == nil)
	if err != nil {
		return
	}
	err = w.Await(5*time.Second, func() bool {
		var row contracts.ReadResult
		_, err := world.Request(ctx, w.Replicas[0], "/v1/op/replica.status", map[string]any{}, &row)
		return err == nil && row.Replica != nil && row.Replica.State == "connected"
	})
	add("replica_feed_reconnected", err == nil)
	response, _, err = owner.request("POST", "approvals/"+offline.ID, map[string]string{"decision": "approve"})
	add("owner_approves_after_replica_reconnect", err == nil && response.StatusCode == 200)
	err = w.Await(5*time.Second, func() bool {
		endpoint := w.Replicas[0]
		endpoint.Token = token
		var row contracts.CommandRecord
		_, err := world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: offline.ID}, &row)
		return err == nil && row.State == contracts.CommandObserved
	})
	effects, _ = w.Effects()
	add("reconnected_approval_executes_once", err == nil && effects[offline.ID] == 1)
	relayAgentParentBoundaries(ctx, w, owner, authority, token, call, command, add)
}

func pendingReplicaApproval(owner *webClient, id string) bool {
	response, raw, err := owner.request("GET", "approvals", nil)
	var out struct {
		Approvals []struct {
			ID string `json:"command_id"`
		} `json:"approvals"`
	}
	if err != nil || response.StatusCode != 200 || json.Unmarshal(raw, &out) != nil {
		return false
	}
	for _, row := range out.Approvals {
		if row.ID == id {
			return true
		}
	}
	return false
}
