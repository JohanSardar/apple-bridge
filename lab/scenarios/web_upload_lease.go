package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadApprovalLease(ctx context.Context, w *world.World, c *webClient, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("approval_authority", false)
		return
	}
	_, token, err := authority.CreateAgent("Bridge Test upload approval", trust.AgentApprove, []contracts.Grant{"messages:write"})
	if err != nil {
		add("approval_agent", false)
		return
	}
	endpoint := w.Gateway
	endpoint.Token = token
	file, err := stageMessageFile(ctx, endpoint, "Bridge Test.txt", []byte("Bridge Test protected bytes"))
	add("agent_upload_staged", err == nil)
	if err != nil {
		return
	}
	args, _ := json.Marshal(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "Bridge Test upload approval", "files": []contracts.MessageFile{file}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args}
	var pending contracts.CommandRecord
	_, err = world.Request(ctx, endpoint, "/v1/op/messages.send", command, &pending)
	add("uploaded_command_awaiting_owner", err == nil && pending.State == contracts.CommandAwaitingApproval)
	ref, ok := pending.UploadReferences[file.UploadID]
	add("durable_command_blob_references_before_approval", ok && contracts.ValidDigest(ref.Digest))
	if !ok {
		return
	}
	paths, _ := filepath.Glob(filepath.Join(w.Gateway.Home, "blobs/upload-leases/*.json"))
	leasePath := ""
	for _, path := range paths {
		raw, _ := os.ReadFile(path)
		var lease map[string]any
		if json.Unmarshal(raw, &lease) == nil && lease["upload_id"] == file.UploadID {
			lease["expires_at"] = time.Now().Add(-time.Second).UTC().Format(time.RFC3339Nano)
			raw, _ = json.Marshal(lease)
			err = os.WriteFile(path, raw, 0600)
			if err == nil {
				leasePath = path
			}
			break
		}
	}
	add("expired_lease_fixture", leasePath != "")
	if leasePath == "" {
		return
	}
	deadline := time.Now().Add(75 * time.Second)
	for time.Now().Before(deadline) {
		if _, err = os.Stat(leasePath); os.IsNotExist(err) {
			break
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(100 * time.Millisecond):
		}
	}
	_, err = os.Stat(leasePath)
	add("abandoned_lease_collected", os.IsNotExist(err))
	alias := filepath.Join(w.Gateway.Home, "blobs/message-uploads", strings.TrimSuffix(filepath.Base(leasePath), ".json"))
	_, err = os.Stat(alias)
	add("expired_upload_alias_removed", os.IsNotExist(err))
	response, _, err := c.request("POST", "approvals/"+command.ID, map[string]string{"decision": "approve"})
	add("owner_approval_after_upload_lease_expiry", err == nil && response.StatusCode == 200)
	var result contracts.CommandRecord
	_, err = world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: command.ID}, &result)
	for deadline := time.Now().Add(20 * time.Second); err == nil && result.State != contracts.CommandObserved && result.Error == nil && time.Now().Before(deadline); {
		select {
		case <-ctx.Done():
			return
		case <-time.After(20 * time.Millisecond):
		}
		_, err = world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: command.ID}, &result)
	}
	add("protected_upload_observed_after_lease_expiry", err == nil && result.State == contracts.CommandObserved)
	effects, err := w.Effects()
	add("protected_upload_exactly_one_effect", err == nil && effects[command.ID] == 1)
}
