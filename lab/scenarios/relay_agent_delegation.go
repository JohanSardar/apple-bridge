package scenarios

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayAgentDelegationBoundaries(ctx context.Context, w *world.World, authority *trust.Authority, agentID string, command func() contracts.Command, add func(string, bool)) {
	p, err := authority.Principal(agentID)
	enrollment, enrollErr := authority.Enrollment()
	if err != nil || enrollErr != nil || enrollment == nil {
		add("delegation_boundary_fixture", false)
		return
	}
	var config world.Config
	raw, readErr := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if readErr != nil || json.Unmarshal(raw, &config) != nil {
		add("delegation_peer_address", false)
		return
	}
	transport := &http.Transport{TLSClientConfig: authority.ClientTLS(enrollment.Fingerprint)}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 5 * time.Second}
	for _, test := range []struct {
		name   string
		change func(map[string]any)
		resign bool
	}{
		{"tampered_level", func(d map[string]any) { d["local_level"] = "approve" }, false},
		{"stale_signature", func(d map[string]any) { d["iat"] = time.Now().Unix() - 61 }, true},
		{"future_signature", func(d map[string]any) { d["iat"] = time.Now().Unix() + 3600 }, true},
		{"invalid_level", func(d map[string]any) { d["local_level"] = "owner" }, true},
		{"read_level_write_scope", func(d map[string]any) { d["local_level"] = "read" }, true},
		{"scope_escalation", func(d map[string]any) { d["local_grants"] = []string{"admin"} }, true},
	} {
		c := command()
		hash, _ := contracts.CommandHash(c)
		header, err := authority.SignDelegation(p, c.ID, hash)
		if err != nil {
			add(test.name, false)
			continue
		}
		parts := strings.Split(header, ".")
		raw, _ := base64.RawURLEncoding.DecodeString(parts[0])
		var d map[string]any
		json.Unmarshal(raw, &d)
		test.change(d)
		raw, _ = json.Marshal(d)
		parts[0] = base64.RawURLEncoding.EncodeToString(raw)
		if test.resign {
			parts[1] = base64.RawURLEncoding.EncodeToString(ed25519.Sign(authority.Certificate.PrivateKey.(ed25519.PrivateKey), raw))
		}
		body, _ := json.Marshal(map[string]any{"method": "submit", "command": c})
		request, err := http.NewRequestWithContext(ctx, "POST", "https://"+config.PeerAddress+"/v1/commands", bytes.NewReader(body))
		if err != nil {
			add(test.name, false)
			continue
		}
		request.Header.Set("X-Apple-Bridge-Delegation", strings.Join(parts, "."))
		response, err := client.Do(request)
		if err != nil {
			add(test.name, false)
			continue
		}
		var result struct {
			Error *contracts.Error `json:"error"`
		}
		err = json.NewDecoder(response.Body).Decode(&result)
		response.Body.Close()
		effects, effectErr := w.Effects()
		add("delegated_agent_"+test.name+"_rejected", err == nil && effectErr == nil && result.Error != nil && result.Error.Code == "unauthorized" && effects[c.ID] == 0)
	}
}
