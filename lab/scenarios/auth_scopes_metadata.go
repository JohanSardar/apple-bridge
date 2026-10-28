package scenarios

import (
	"context"
	"encoding/json"
	"maps"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runAuthScopesMetadata(ctx context.Context, w *world.World, add func(string, bool)) {
	expected := map[string]bool{}
	for _, source := range contracts.Sources {
		group, ok := contracts.GrantsForSource(source)
		add("registered_grants_"+string(source), ok && len(group.Grants) > 0)
		for _, grant := range group.Grants {
			expected[string(grant)] = true
		}
	}
	add("new_source_registered", expected["labdummy:read"] && expected["labdummy:write"])
	var first map[string]bool
	for _, path := range []string{"/.well-known/oauth-protected-resource", "/.well-known/oauth-protected-resource/mcp", "/.well-known/oauth-authorization-server"} {
		status, raw, _ := agentRequest(ctx, w.Gateway.URL, "", "GET", path, nil)
		var metadata struct {
			Scopes []string `json:"scopes_supported"`
		}
		decodeErr := json.Unmarshal(raw, &metadata)
		add("metadata_document_"+path, status == 200 && decodeErr == nil && len(metadata.Scopes) > 0)
		scopes := map[string]bool{}
		for _, scope := range metadata.Scopes {
			scopes[scope] = true
		}
		add("all_registered_grants_"+path, maps.Equal(scopes, expected))
		if first == nil {
			first = scopes
		} else {
			add("equal_scope_sets_"+path, maps.Equal(first, scopes))
		}
	}
}
