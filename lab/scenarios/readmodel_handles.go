package scenarios

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"strings"
	"time"
)

func rmHandles(ctx context.Context, w *world.World, add func(string, bool)) {
	digest := strings.Repeat("a", 64)
	add("handle_avatar_fixture", w.ControlAction("/external-change", map[string]any{"key": "rm-contact-0", "collection": "contacts.contact", "data": map[string]any{"given_name": "Alpha", "emails": []any{map[string]any{"value": "synthetic-0@example.invalid"}}, "thumbnail_image": map[string]any{"digest": digest, "state": "present"}}}) == nil)
	add("handle_avatar_capture", w.FlushCaptures(20*time.Second) == nil)
	aliases, _ := gatewayAliases(w)
	id := aliases[aliasKey(contracts.Messages, "messages.handle", "rm-handle-0")]
	restricted := w.Gateway
	restricted.Token, _ = w.ScopedToken(w.Gateway, "messages:read")
	for _, protocol := range []string{"rest", "mcp"} {
		c := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		var out contracts.HandlesResult
		err := c.Call(ctx, "messages.handles", contracts.HandlesQuery{IDs: []string{id, id, "unknown"}}, &out)
		valid := err == nil && len(out.Handles) == 1
		if valid {
			h := out.Handles[0]
			valid = h.HandleID == id && h.DisplayName == "Alpha" && h.GivenName == "Alpha" && h.FamilyName == "" && h.Initials == "A" && h.ContactID != nil && h.Avatar != nil && h.Avatar.Digest == digest && h.Avatar.Variant == "original"
		}
		add(protocol+"_batched_handle_name_avatar_dedup", valid)
		var hidden contracts.HandlesResult
		err = (scripted.Client{Endpoint: restricted, Protocol: protocol}).Call(ctx, "messages.handles", contracts.HandlesQuery{IDs: []string{id}}, &hidden)
		valid = err == nil && len(hidden.Handles) == 1
		if valid {
			h := hidden.Handles[0]
			valid = h.DisplayName == "synthetic-0@example.invalid" && h.ContactID == nil && h.Avatar == nil && h.GivenName == "" && h.FamilyName == ""
		}
		add(protocol+"_handle_contact_visibility", valid)
		add(protocol+"_handles_input_budget", c.Call(ctx, "messages.handles", contracts.HandlesQuery{IDs: make([]string, 501)}, nil) != nil)
	}
}
