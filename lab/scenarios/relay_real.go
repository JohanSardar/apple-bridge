package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRelayReal(ctx context.Context, s Scenario, e Evidence) Evidence {
	e.RunID = contracts.NewID("relay-real")
	e.Hosts = []string{"air", "ubuntu", "ab-relay"}
	e.NetworkPath = "Ubuntu public HTTPS through ab-relay ciphertext tunnel to isolated Mac gateway"
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("isolated_mac_gateway", err == nil)
	if err != nil {
		return e
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
	add("relay_enrollment_dns01_public_tls_and_60s_sse", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
		log, _ := os.ReadFile(filepath.Join(w.Dir, "gateway.log"))
		for _, line := range strings.Split(string(log), "\n") {
			if strings.Contains(line, "remote setup failed:") {
				fmt.Fprintln(os.Stderr, line)
			}
		}
		return e
	}
	data, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "status.json"))
	var status remote.Status
	json.Unmarshal(data, &status)
	u, err := url.Parse(status.URL)
	add("relay_backend_on", err == nil && status.State == "on" && status.Backend == "relay")
	if err != nil {
		return e
	}
	if s.ID == "CONN-CLAUDE-CODE" {
		token, err := exec.CommandContext(ctx, "bin/bridge", "token", "create", "--remote", "--scope", "admin", "--home", w.Gateway.Home).Output()
		add("remote_token", err == nil)
		if err != nil {
			return e
		}
		config, _ := json.Marshal(map[string]any{"mcpServers": map[string]any{"apple-bridge-relay-test": map[string]any{"type": "http", "url": status.URL + "/mcp", "headers": map[string]string{"Authorization": "Bearer " + strings.TrimSpace(string(token))}}}})
		err = os.WriteFile(filepath.Join(w.Dir, ".mcp.json"), config, 0600)
		add("isolated_claude_configuration", err == nil)
		if err != nil {
			return e
		}
		callCtx, callCancel := context.WithTimeout(ctx, 90*time.Second)
		defer callCancel()
		call := exec.CommandContext(callCtx, "claude", "-p", "Call mcp__apple-bridge-relay-test__records_query exactly once with limit 1. Do not use any other tool. Report only success or failure.", "--mcp-config", filepath.Join(w.Dir, ".mcp.json"), "--strict-mcp-config", "--allowedTools", "mcp__apple-bridge-relay-test__records_query", "--output-format", "stream-json", "--verbose", "--max-turns", "3")
		call.Dir = w.Dir
		output, err := call.Output()
		add("claude_code_real_tool_call", err == nil && bytes.Contains(output, []byte(`"name":"mcp__apple-bridge-relay-test__records_query"`)) && bytes.Contains(output, []byte(`"type":"tool_result"`)) && !bytes.Contains(output, []byte(`"is_error":true`)))

		return e
	}
	ip := remotePublicChecks(ctx, status, u, add)
	if ip == "" {
		return e
	}
	binary := filepath.Join(w.Dir, "remoteprobe")
	build := exec.CommandContext(ctx, "go", "build", "-p", "2", "-o", binary, "./lab/remoteprobe")
	build.Env = append(os.Environ(), "GOOS=linux", "GOARCH=amd64", "CGO_ENABLED=0")
	err = build.Run()
	add("external_probe_build", err == nil)
	if err != nil {
		return e
	}
	dir := "/tmp/" + e.RunID
	err = exec.CommandContext(ctx, "ssh", "ubuntu", "mkdir", "-m", "700", dir).Run()
	add("external_probe_directory", err == nil)
	if err != nil {
		return e
	}
	defer exec.Command("ssh", "ubuntu", "rm", "-rf", dir).Run()
	err = exec.CommandContext(ctx, "scp", binary, "ubuntu:"+dir+"/probe").Run()
	add("external_probe_copy", err == nil)
	if err != nil {
		return e
	}
	token, err := exec.CommandContext(ctx, "bin/bridge", "token", "create", "--remote", "--scope", "admin", "--home", w.Gateway.Home).Output()
	add("remote_token", err == nil)
	if err != nil {
		return e
	}
	input, _ := json.Marshal(map[string]any{"URL": status.URL, "IP": ip, "Remote": strings.TrimSpace(string(token)), "Local": w.Gateway.Token})
	probe := exec.CommandContext(ctx, "ssh", "ubuntu", dir+"/probe")
	probe.Stdin = bytes.NewReader(input)
	output, err := probe.Output()
	add("external_probe_completed", err == nil)
	var result struct {
		Checks map[string]bool `json:"checks"`
	}
	err = json.Unmarshal(output, &result)
	add("external_probe_results", err == nil && len(result.Checks) > 0)
	for name, ok := range result.Checks {
		add(name, ok)
	}

	if s.ID == "CONN-RELAY-CLAUDE" {

		config, _ := json.Marshal(map[string]any{"mcpServers": map[string]any{"apple-bridge-relay-test": map[string]any{"type": "http", "url": status.URL + "/mcp", "headers": map[string]string{"Authorization": "Bearer " + strings.TrimSpace(string(token))}}}})
		err = os.WriteFile(filepath.Join(w.Dir, ".mcp.json"), config, 0600)
		if err == nil {
			err = os.MkdirAll(filepath.Join(w.Dir, ".claude"), 0700)
		}
		if err == nil {
			err = os.WriteFile(filepath.Join(w.Dir, ".claude", "settings.local.json"), []byte(`{"enableAllProjectMcpServers":true}`), 0600)
		}
		add("isolated_claude_configuration", err == nil)
		command := exec.CommandContext(ctx, "claude", "mcp", "list")
		command.Dir = w.Dir
		command.Env = append(os.Environ(), "CLAUDE_CONFIG_DIR="+filepath.Join(w.Dir, "claude-config"))
		server, _ := json.Marshal(map[string]any{"type": "http", "url": status.URL + "/mcp", "headers": map[string]string{"Authorization": "Bearer " + strings.TrimSpace(string(token))}})
		register := exec.CommandContext(ctx, "claude", "mcp", "add-json", "--scope", "user", "apple-bridge-relay-test", string(server))
		register.Dir, register.Env = command.Dir, command.Env
		err = register.Run()
		add("isolated_claude_registration", err == nil)
		output, err := command.CombinedOutput()
		add("claude_code_http_connector_connected", err == nil && strings.Contains(string(output), "apple-bridge-relay-test") && strings.Contains(string(output), "Connected"))
		if err != nil || !strings.Contains(string(output), "Connected") {
			fmt.Fprintln(os.Stderr, "isolated Claude connection:", string(output))
		}

	}

	client := &http.Client{Timeout: 90 * time.Second}
	_, err = remote.ProbeHTTP(ctx, client, status.URL, 60*time.Second)
	add("trusted_public_oauth_mcp_and_60s_sse", err == nil)
	return e
}
