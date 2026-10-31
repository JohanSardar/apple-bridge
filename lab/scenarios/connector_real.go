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
	enroll, err := exec.CommandContext(ctx, "bin/bridge", "web", "enroll", "--public", "--home", w.Gateway.Home).Output()
	link := strings.SplitN(strings.TrimSpace(string(enroll)), "\n", 2)[0]
	add("passkey_enrollment_link", err == nil && strings.HasPrefix(link, "https://"))
	if err != nil {
		return finish()
	}
	home, _ := os.UserHomeDir()
	handoff := filepath.Join(home, "Library", "Application Support", "AppleBridgeDev", "connector-"+strings.ToLower(s.ID)+".json")
	payload, _ := json.Marshal(map[string]string{"connector_url": status.URL + "/mcp", "enroll_url": link})
	os.MkdirAll(filepath.Dir(handoff), 0o700)
	err = os.WriteFile(handoff, payload, 0o600)
	add("handoff_written", err == nil)
	defer os.Remove(handoff)
	start := time.Now()
	authority, err := trust.Open(w.Gateway.Home)
	add("authority", err == nil)
	if err != nil {
		return finish()
	}
	used := false
	deadline := time.Now().Add(25 * time.Minute)
	for !used && time.Now().Before(deadline) && ctx.Err() == nil {
		agents, _ := authority.Agents()
		for _, a := range agents {
			if a.CreatedAt.After(start) && a.LastUsed != nil && a.LastUsed.After(a.CreatedAt) {
				used = true
			}
		}
		if !used {
			time.Sleep(2 * time.Second)
		}
	}
	add("oauth_agent_authorized_and_called_tool", used)
	return finish()
}
