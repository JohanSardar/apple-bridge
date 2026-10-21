package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"net"
	"net/url"
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

func runRelayConnect(ctx context.Context, s Scenario, level string) Evidence {
	return runRelayWorld(ctx, s, level, false)
}
func runRelayReplica(ctx context.Context, s Scenario, level string) Evidence {
	return runRelayWorld(ctx, s, level, true)
}
func runRelayWorld(ctx context.Context, s Scenario, level string, native bool) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("relay"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air", "ubuntu"}, NetworkPath: "public Funnel; pinned public DNS through CONNECT egress; no peer or SSH tunnel"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	before, err := exec.CommandContext(ctx, "/opt/homebrew/bin/tailscale", "serve", "status", "--json").Output()
	add("serve_config_before", err == nil)
	if err != nil {
		return
	}
	var w *world.World
	if native {
		w, err = world.NewReal(ctx, "real", 0, world.BlobOptions{Scope: "admin"})
	} else {
		w, err = world.New(ctx, "sim", 0, 3, world.BlobOptions{Scope: "admin"})
	}
	add("isolated_gateway", err == nil)
	if err != nil {
		dir := filepath.Join("lab/evidence/private/relay", e.RunID)
		os.MkdirAll(dir, 0700)
		os.WriteFile(filepath.Join(dir, "gateway-start-error.log"), []byte(err.Error()), 0600)
		e.Assertions[len(e.Assertions)-1].Detail = "private gateway-start-error.log"
		return
	}
	defer w.Close()
	privateDir := filepath.Join("lab/evidence/private/relay", e.RunID)
	os.MkdirAll(privateDir, 0700)
	os.WriteFile(filepath.Join(privateDir, "serve-before.json"), before, 0600)
	defer func() {
		dir := filepath.Join("lab/evidence/private/relay", e.RunID)
		os.MkdirAll(dir, 0700)
		for _, name := range []string{"gateway.log", "relay-route.log", "relay-replica.log", "relay-probe.log", "relay-start-error.log", "relay-probe-build.log", "relay-snapshot-status.json", "relay-blob-call-error.log", "post-burst-blob-error.log"} {
			if data, err := os.ReadFile(filepath.Join(w.Dir, name)); err == nil {
				os.WriteFile(filepath.Join(dir, name), data, 0600)
			}
		}
	}()
	defer func() {
		bounded, cancel := context.WithTimeout(context.Background(), 8*time.Second)
		defer cancel()
		if lease, err := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "lease.json")); err == nil {
			os.WriteFile(filepath.Join(privateDir, "lease.json"), lease, 0600)
		}
		err := remote.Command(bounded, w.Gateway.Home, "off", "", "", &bytes.Buffer{})
		if err != nil {
			retry, stop := context.WithTimeout(context.Background(), 12*time.Second)
			err = remote.Command(retry, w.Gateway.Home, "off", "", "", &bytes.Buffer{})
			stop()
		}
		add("remote_off", err == nil)
		verify, stop := context.WithTimeout(context.Background(), 3*time.Second)
		defer stop()
		after, err := exec.CommandContext(verify, "/opt/homebrew/bin/tailscale", "serve", "status", "--json").Output()
		add("serve_config_byte_identical_after_off", err == nil && bytes.Equal(before, after))
		e.Passed = len(e.Assertions) > 0
		for _, assertion := range e.Assertions {
			e.Passed = e.Passed && assertion.Passed
		}
	}()
	if native {
		err = relayNativeReady(ctx, w)
		add("signed_native_helper_reminders_capture_ready", err == nil)
		if err != nil {
			return
		}
	}
	bounded, cancel := context.WithTimeout(ctx, 20*time.Second)
	err = remote.Command(bounded, w.Gateway.Home, "on", "system", "bridge", &bytes.Buffer{})
	cancel()
	add("remote_on_system", err == nil)
	if err != nil {
		return
	}
	var status remote.Status
	raw, err := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "status.json"))
	if err != nil {
		add("remote_status", false)
		return
	}
	json.Unmarshal(raw, &status)
	u, err := url.Parse(status.URL)
	add("system_backend_port_10000", err == nil && status.Backend == "system" && u.Port() == "10000")
	if err != nil {
		return
	}
	remoteMappingChecks(ctx, before, u, add)
	encoded, err := exec.CommandContext(ctx, "bin/bridge", "pair", "--via-remote", "--scope", "reminders:write,system:read", "--home", w.Gateway.Home).Output()
	add("pair_via_remote_cli", err == nil)
	if err != nil {
		return
	}
	for _, flag := range []string{"--peer-addr", "--dial-replica"} {
		_, invalid := exec.CommandContext(ctx, "bin/bridge", "pair", "--via-remote", flag, "127.0.0.1:7443", "--home", w.Gateway.Home).Output()
		add("remote_pair_rejects_"+strings.TrimPrefix(flag, "--"), invalid != nil)
	}
	code, err := trust.DecodePairCode(strings.TrimSpace(string(encoded)))
	add("remote_code_public_url_and_gateway_pin", err == nil && code.Remote && code.Address == status.URL && len(code.Fingerprint) == 64)
	if err != nil {
		return
	}
	var cfg world.Config
	raw, err = os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err == nil {
		err = json.Unmarshal(raw, &cfg)
	}
	peerHost, _, peerErr := net.SplitHostPort(cfg.PeerAddress)
	add("gateway_peer_loopback_only", err == nil && peerErr == nil && net.ParseIP(peerHost).IsLoopback())
	ip := remotePublicIP(ctx, u, add)
	if ip == "" || !relayIngressReady(ctx, status, u, ip, &e, add) {
		return
	}
	remotePublicHTTPChecks(ctx, status, u, ip, add)
	probe := filepath.Join(w.Dir, "relayprobe")
	build := exec.CommandContext(ctx, "go", "build", "-p", "2", "-o", probe, "./lab/relayprobe")
	build.Env = append(os.Environ(), "GOOS=linux", "GOARCH=amd64", "CGO_ENABLED=0")
	output, err := build.CombinedOutput()
	add("relay_probe_cross_compiled_on_air", err == nil)
	if err != nil {
		os.WriteFile(filepath.Join(w.Dir, "relay-probe-build.log"), output, 0600)
		return
	}
	err = w.AddRelayReplica("reminders:write,system:read", ip, probe)
	add("ubuntu_joins_and_starts_through_remote", err == nil)
	if err != nil {
		os.WriteFile(filepath.Join(w.Dir, "relay-start-error.log"), []byte(err.Error()), 0600)
		e.Assertions[len(e.Assertions)-1].Detail = "private relay-start-error.log"
		return
	}
	replica := w.Replicas[0]
	var result relayQuery
	gateway, err := relayGatewaySnapshot(ctx, w.Gateway)
	add("gateway_snapshot_reference", err == nil)
	if err != nil {
		return
	}
	snapshotChecks := map[string]bool{"gateway_read": true}
	err = w.Await(20*time.Second, func() bool {
		err = w.RelayProbe("query", map[string]any{"URL": replica.URL, "Host": replica.Host, "Token": replica.Token, "Home": replica.Home, "Collection": "reminders.reminder", "ProxyReady": filepath.Join(w.Dir, "relay-route", "ready.json")}, &result)
		snapshotChecks["replica_read"] = err == nil
		if err != nil {
			return false
		}
		snapshotChecks["connected"] = result.Status.State == "connected"
		snapshotChecks["count_equal"] = result.Count == gateway.Count && (native || result.Count == 3)
		snapshotChecks["digest_equal"] = result.Digest == gateway.Digest
		snapshotChecks["epoch_equal"] = result.Cursor.Epoch == gateway.Cursor.Epoch
		snapshotChecks["gateway_watermark_reached"] = result.Cursor.Sequence >= gateway.Cursor.Sequence
		for _, ok := range snapshotChecks {
			if !ok {
				return false
			}
		}
		return true
	})
	if err != nil {
		raw, _ := json.Marshal(snapshotChecks)
		os.WriteFile(filepath.Join(w.Dir, "relay-snapshot-status.json"), raw, 0600)
	}
	add("public_relay_snapshot_converges", err == nil && result.Status.Snapshots > 0 && result.Status.SnapshotPages > 0)
	add("public_tls_plus_pinned_gateway_identity", result.Enrollment != nil && result.Enrollment.Remote && result.Enrollment.Address == status.URL && result.Enrollment.Fingerprint == code.Fingerprint)
	add("join_and_feed_forced_through_public_relay", result.PublicConnections >= 2)
	e.Metrics["snapshot_records"] = result.Count
	e.Metrics["snapshot_pages"] = result.Status.SnapshotPages
	e.Metrics["public_connections"] = result.PublicConnections
	if native {
		delete(e.Metrics, "snapshot_records")
		delete(e.Metrics, "snapshot_pages")
		e.Metrics["snapshot_converged"] = err == nil
	}
	if err == nil && s.ID == "WEB-RELAY-COMMAND" {
		relayCommandChecks(w, &e, add)
	}
	if err == nil && native && s.ID == "WEB-RELAY-REPLICA-BLOBS" {
		relayNativeBlobChecks(ctx, w, &e, add)
		return
	}
	if err == nil && (native || s.ID == "WEB-RELAY-PUBLIC-AGENTS") && result.Enrollment != nil {
		relayPublicAgentChecks(ctx, w, &e, add, *result.Enrollment)
	}
	if err == nil && native && s.ID == "WEB-RELAY-REPLICA" {
		relayPostBurstChecks(ctx, w, &e, add)
	}
	if err == nil && result.Enrollment != nil {
		relaySecurityChecks(w, &e, add, result.Enrollment)
	}
	return
}

type relayQuery struct {
	Count             int                     `json:"count"`
	Digest            string                  `json:"digest"`
	Cursor            contracts.Cursor        `json:"cursor"`
	Status            contracts.ReplicaStatus `json:"status"`
	Enrollment        *trust.Enrollment       `json:"enrollment"`
	PublicConnections int                     `json:"public_connections"`
}
