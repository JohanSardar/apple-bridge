package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
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

func runRemote(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("remote"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air", "ubuntu"}, NetworkPath: "public DNS pinned Funnel relay; real system Tailscale; isolated synthetic gateway"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	before, err := exec.CommandContext(ctx, "/opt/homebrew/bin/tailscale", "serve", "status", "--json").Output()
	add("serve_config_before", err == nil)
	if err != nil {
		return e
	}
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("isolated_gateway", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	off := func() error {
		bounded, cancel := context.WithTimeout(context.Background(), 8*time.Second)
		defer cancel()
		return remote.Command(bounded, w.Gateway.Home, "off", "", "", &bytes.Buffer{})
	}
	defer off()
	bounded, cancel := context.WithTimeout(ctx, 20*time.Second)
	err = remote.Command(bounded, w.Gateway.Home, "on", "", "bridge", &bytes.Buffer{})
	cancel()
	add("remote_on_system", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
		return e
	}
	var status remote.Status
	data, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "status.json"))
	json.Unmarshal(data, &status)
	u, _ := url.Parse(status.URL)
	add("system_backend_port_10000", status.Backend == "system" && u.Port() == "10000")
	statusOutput, statusErr := exec.CommandContext(ctx, "bin/bridge", "remote", "status", "--home", w.Gateway.Home).Output()
	var cliStatus remote.Status
	json.Unmarshal(statusOutput, &cliStatus)
	add("remote_status_cli_state_backend_url", statusErr == nil && cliStatus.State == "on" && cliStatus.Backend == "system" && cliStatus.URL == status.URL)

	remoteProxyChecks(ctx, w.Gateway.Home, status, u, add)
	remoteMappingChecks(ctx, before, u, add)
	ip := remotePublicChecks(ctx, status, u, add)
	if ip == "" {
		return e
	}
	dir := "/tmp/ablab/" + e.RunID
	if err = exec.CommandContext(ctx, "ssh", "ubuntu", "mkdir", "-p", dir).Run(); err != nil {
		add("ubuntu_probe_dir", false)
		return e
	}
	defer exec.Command("ssh", "ubuntu", "rm", "-rf", dir).Run()
	build := exec.CommandContext(ctx, "go", "build", "-p", "2", "-o", filepath.Join(w.Dir, "remoteprobe"), "./lab/remoteprobe")
	build.Env = append(os.Environ(), "GOOS=linux", "GOARCH=amd64", "CGO_ENABLED=0")
	output, err := build.CombinedOutput()
	add("probe_cross_compiled_on_air", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = string(output)
		return e
	}
	err = exec.CommandContext(ctx, "scp", filepath.Join(w.Dir, "remoteprobe"), "ubuntu:"+dir+"/remoteprobe").Run()
	add("probe_copied_to_ubuntu", err == nil)
	if err != nil {
		return e
	}
	tokenBytes, err := exec.CommandContext(ctx, "bin/bridge", "token", "create", "--remote", "--scope", "admin", "--home", w.Gateway.Home).Output()
	add("remote_token_cli", err == nil)
	if err != nil {
		return e
	}
	token := strings.TrimSpace(string(tokenBytes))
	probe := func(off bool) {
		input, _ := json.Marshal(map[string]any{"URL": status.URL, "IP": ip, "Remote": token, "Local": w.Gateway.Token, "Off": off})
		cmd := exec.CommandContext(ctx, "ssh", "ubuntu", dir+"/remoteprobe")
		cmd.Stdin = bytes.NewReader(input)
		output, err := cmd.Output()
		add(fmt.Sprintf("ubuntu_probe_off_%t", off), err == nil)
		if err != nil {
			return
		}
		var result struct {
			Checks  map[string]bool `json:"checks"`
			Latency Latency         `json:"latency"`
		}
		if json.Unmarshal(output, &result) != nil {
			add("probe_json", false)
			return
		}
		for name, passed := range result.Checks {
			add(name, passed)
		}
		if !off {
			e.Latency["ubuntu_public_rest_records_query"] = result.Latency
		}
	}
	probe(false)
	err = w.RestartGateway()
	add("gateway_restart", err == nil)
	restored := false
	deadline := time.Now().Add(10 * time.Second)
	for err == nil && time.Now().Before(deadline) {
		var restoredStatus remote.Status
		data, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "status.json"))
		if json.Unmarshal(data, &restoredStatus) == nil && restoredStatus.State == "on" && restoredStatus.Backend == "system" && restoredStatus.URL == status.URL {
			restored = true
			break
		}
		time.Sleep(100 * time.Millisecond)
	}
	add("remote_backend_url_restored", restored)
	probe(false)
	start := time.Now()
	err = off()
	add("remote_off", err == nil)
	probe(true)
	e.Latency["remote_off_public_unreachable"] = Latency{Samples: 1, P50: float64(time.Since(start)) / float64(time.Millisecond), P95: float64(time.Since(start)) / float64(time.Millisecond), Max: float64(time.Since(start)) / float64(time.Millisecond)}
	add("off_within_one_second", time.Since(start) <= time.Second)
	after, err := exec.CommandContext(ctx, "/opt/homebrew/bin/tailscale", "serve", "status", "--json").Output()
	add("serve_config_byte_identical_after_off", err == nil && bytes.Equal(before, after))
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
