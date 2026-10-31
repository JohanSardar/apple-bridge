package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runConnectorReal(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: contracts.NewID("connector-real"), Hosts: []string{"air", "ab-relay"}, NetworkPath: "browser connector through ab-relay ciphertext tunnel to isolated Mac gateway"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	finish := func() Evidence {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
		return e
	}
	webBuilt := true
	for _, args := range [][]string{{"install", "--frozen-lockfile"}, {"build:release"}} {
		command := exec.CommandContext(ctx, "pnpm", args...)
		command.Dir = "web"
		command.Stdout = os.Stderr
		command.Stderr = os.Stderr
		if command.Run() != nil {
			webBuilt = false
			break
		}
	}
	add("approval_web_built", webBuilt)
	if !webBuilt {
		return finish()
	}
	err := world.Build(ctx, "sim")
	add("gateway_embeds_approval_web", err == nil)
	if err != nil {
		return finish()
	}
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("isolated_mac_gateway", err == nil)
	if err != nil {
		return finish()
	}
	defer w.Close()
	defer func() {
		bounded, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		remote.Command(bounded, w.Gateway.Home, "off", "", "", &bytes.Buffer{})
	}()
	bounded, cancel := context.WithTimeout(ctx, 7*time.Minute)
	err = remote.Command(bounded, w.Gateway.Home, "on", "relay", "bridge", &bytes.Buffer{})
	cancel()
	add("relay_public_origin", err == nil)
	if err != nil {
		return finish()
	}
	data, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "status.json"))
	var status remote.Status
	json.Unmarshal(data, &status)
	home, _ := os.UserHomeDir()
	handoff := filepath.Join(home, "Library", "Application Support", "AppleBridgeDev", "connector-"+strings.ToLower(s.ID)+".json")
	payload, _ := json.Marshal(map[string]string{"connector_url": status.URL + "/mcp", "approval_home": w.Gateway.Home, "approval_mode": "owner-local Mac socket; runner matches each request code"})
	os.MkdirAll(filepath.Dir(handoff), 0o700)
	err = os.WriteFile(handoff, payload, 0o600)
	add("handoff_written", err == nil)
	defer os.Remove(handoff)
	start := time.Now()
	approved := false
	macApprovalOK := true
	authority, err := trust.Open(w.Gateway.Home)
	add("authority", err == nil)
	if err != nil {
		return finish()
	}
	used := false
	deadline := time.Now().Add(5 * time.Minute)
	for !used && time.Now().Before(deadline) && ctx.Err() == nil {
		var pending bytes.Buffer
		if bridgeweb.ConnectorCommand(ctx, w.Gateway.Home, "pending", "", "", &pending) == nil {
			var requests []bridgeweb.ConnectorApproval
			if json.Unmarshal(pending.Bytes(), &requests) == nil {
				for _, request := range requests {
					expected := "claude"
					if s.ID == "CONN-CHATGPT" {
						expected = "chatgpt"
					}
					name := strings.ToLower(request.ClientName)
					if strings.Contains(name, expected) || expected == "chatgpt" && strings.Contains(name, "openai") {
						err := bridgeweb.ConnectorCommand(ctx, w.Gateway.Home, "allow", request.ID, request.Code, &bytes.Buffer{})
						approved = approved || err == nil
						macApprovalOK = macApprovalOK && err == nil
					}
				}
			}
		}
		agents, _ := authority.Agents()
		for _, a := range agents {
			if a.CreatedAt.After(start) && a.LastUsed != nil && a.LastUsed.After(a.CreatedAt) {
				used = true
			}
		}
		if !used {
			select {
			case <-ctx.Done():
			case <-time.After(2 * time.Second):
			}
		}
	}
	add("owner_local_mac_approval", approved && macApprovalOK)
	add("oauth_agent_authorized_and_called_tool", used)
	return finish()
}
