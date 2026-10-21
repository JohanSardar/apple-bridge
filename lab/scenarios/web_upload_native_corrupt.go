package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadNativeCorrupt(ctx context.Context, w *world.World, c *webClient, run string, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("corruption_authority", false)
		return
	}
	_, token, err := authority.CreateAgent("Bridge Test corrupt upload", trust.AgentApprove, []contracts.Grant{"messages:write"})
	if err != nil {
		add("corruption_agent", false)
		return
	}
	endpoint := w.Gateway
	endpoint.Token = token
	data := []byte("[ab-test " + run + "] corrupt attachment")
	file, err := stageMessageFile(ctx, endpoint, "ab-"+run+".txt", data)
	add("corruption_fixture_uploaded", err == nil)
	if err != nil {
		return
	}
	raw, _ := json.Marshal(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "[ab-test " + run + "]", "files": []contracts.MessageFile{file}})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: raw}
	var pending contracts.CommandRecord
	_, err = world.Request(ctx, endpoint, "/v1/op/messages.send", command, &pending)
	add("corruption_fixture_admitted_for_approval", err == nil && pending.State == contracts.CommandAwaitingApproval)
	ref := pending.UploadReferences[file.UploadID]
	if !contracts.ValidDigest(ref.Digest) {
		return
	}
	path := filepath.Join(w.Gateway.Home, "blobs", "sha256", ref.Digest[:2], ref.Digest[2:])
	err = os.WriteFile(path, bytes.Repeat([]byte("x"), len(data)), 0600)
	add("same_size_blob_corruption_injected", err == nil)
	if err != nil {
		return
	}
	c.request("POST", "approvals/"+command.ID, map[string]string{"decision": "approve"})
	var result contracts.CommandRecord
	err = w.Await(5*time.Second, func() bool {
		var problem error
		result, problem = uploadGatewayCommand(ctx, w.Gateway.Home, command.ID)
		return problem == nil && result.State == contracts.CommandRejected
	})
	prepared := false
	for _, event := range result.History {
		prepared = prepared || event.State == contracts.CommandPrepared || event.State == contracts.CommandDispatched
	}
	add("changed_digest_rejected_before_native_prepare", err == nil && !prepared)
	effects, problem := w.Effects()
	add("corrupt_upload_has_zero_native_effects", problem == nil && effects[command.ID] == 0)
	add("corrupt_staging_removed", uploadCommandStagingEmpty(w))
}
