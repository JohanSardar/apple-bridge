package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"maps"
	"os"
	"sort"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runAuthScopesMetadata(ctx context.Context, w *world.World, add func(string, bool)) {
	expected := map[string]bool{}
	groups, err := frozenSourceGrants()
	add("frozen_source_grants", err == nil)
	if err != nil {
		return
	}
	for _, group := range groups {
		source := group.Source
		_, err := w.ScopedToken(w.Gateway, string(group.Grants[0])+","+string(group.Grants[1]))
		ok := err == nil
		add("registered_grants_"+string(source), ok && len(group.Grants) > 0)
		for _, grant := range group.Grants {
			if group.Agent && (grant == contracts.Grant(string(source)+":read") || grant == contracts.Grant(string(source)+":write")) {
				expected[string(grant)] = true
			}
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

type sourceGrantFixture struct {
	Source string
	Grants []contracts.Grant
	Agent  bool
}

func frozenSourceGrants() ([]sourceGrantFixture, error) {
	raw, err := os.ReadFile("contracts/golden/oauth.json")
	if err != nil {
		return nil, err
	}
	var corpus struct {
		Exchanges []struct {
			Response string `json:"response"`
		} `json:"exchanges"`
	}
	if err = json.Unmarshal(raw, &corpus); err != nil {
		return nil, err
	}
	groups := map[string]bool{"system": false, "labdummy": true}
	for _, exchange := range corpus.Exchanges {
		var metadata struct {
			Scopes []string `json:"scopes_supported"`
		}
		if err = json.Unmarshal([]byte(exchange.Response), &metadata); err != nil {
			continue
		}
		for _, scope := range metadata.Scopes {
			source, suffix, ok := strings.Cut(scope, ":")
			if ok && (suffix == "read" || suffix == "write") {
				groups[source] = true
			}
		}
	}
	if len(groups) < 3 {
		return nil, fmt.Errorf("frozen OAuth source grants missing")
	}
	sources := make([]string, 0, len(groups))
	for source := range groups {
		sources = append(sources, source)
	}
	sort.Strings(sources)
	fixtures := make([]sourceGrantFixture, 0, len(sources))
	for _, source := range sources {
		fixtures = append(fixtures, sourceGrantFixture{Source: source, Agent: groups[source], Grants: []contracts.Grant{contracts.Grant(source + ":read"), contracts.Grant(source + ":write")}})
	}
	return fixtures, nil
}
