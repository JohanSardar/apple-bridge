package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayAgentParentBoundaries(ctx context.Context, w *world.World, owner *webClient, replica *trust.Authority, token string, call func(string, contracts.Command) (int, contracts.CommandRecord), command func() contracts.Command, add func(string, bool)) {
	gateway, err := trust.Open(w.Gateway.Home)
	enrollment, enrollErr := replica.Enrollment()
	if err != nil || enrollErr != nil || enrollment == nil {
		add("parent_scope_fixture", false)
		return
	}
	scoped, revoked := command(), command()
	code, row := call(token, scoped)
	add("scope_fixture_pending", code == 200 && row.State == contracts.CommandAwaitingApproval)
	code, row = call(token, revoked)
	add("parent_revocation_fixture_pending", code == 200 && row.State == contracts.CommandAwaitingApproval)
	err = gateway.ChangeScope(enrollment.ReplicaID, []contracts.Grant{"reminders:read", "system:read"})
	add("parent_scope_narrowed", err == nil)
	parent, err := gateway.Principal(enrollment.ReplicaID)
	if err != nil {
		add("updated_parent_principal", false)
		return
	}
	err = w.Await(5*time.Second, func() bool {
		var status contracts.ReadResult
		_, err := world.Request(ctx, w.Replicas[0], "/v1/op/replica.status", map[string]any{}, &status)
		return err == nil && status.Replica != nil && status.Replica.State == "connected" && status.Replica.Cursor.SubscriptionRevision == parent.Revision
	})
	add("narrowed_subscription_connected", err == nil)
	response, raw, err := owner.request("POST", "approvals/"+scoped.ID, map[string]string{"decision": "approve"})
	json.Unmarshal(raw, &row)
	effects, _ := w.Effects()
	add("fresh_parent_scope_rejects_pending_write", err == nil && response.StatusCode == 200 && row.State == contracts.CommandRejected && effects[scoped.ID] == 0)
	err = gateway.Revoke(enrollment.ReplicaID)
	add("parent_replica_revoked", err == nil)
	response, raw, err = owner.request("POST", "approvals/"+revoked.ID, map[string]string{"decision": "approve"})
	json.Unmarshal(raw, &row)
	effects, _ = w.Effects()
	add("revoked_parent_rejects_pending_agent_approval", err == nil && response.StatusCode == 200 && row.State == contracts.CommandRejected && effects[revoked.ID] == 0)
}
