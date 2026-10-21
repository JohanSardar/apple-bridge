package scenarios

import (
	"context"
	"net/http"
	"net/http/httptest"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
)

func runRelayAuth(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("relay"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, "sim", 1, 2)
	add("paired_world", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	authority, err := trust.Open(w.Gateway.Home)
	add("gateway_identity", err == nil)
	if err != nil {
		return e
	}
	feed := &replication.Server{Authority: authority}
	fixture := httptest.NewTLSServer(feed.RelayHandler())
	defer fixture.Close()
	conn, _, err := websocket.Dial(ctx, strings.Replace(fixture.URL, "https:", "wss:", 1)+"/v1/feed", &websocket.DialOptions{HTTPClient: fixture.Client()})
	add("remote_websocket_without_client_tls_certificate", err == nil)
	if err != nil {
		return e
	}
	replica, err := trust.Open(w.Replicas[0].Home)
	add("replica_identity", err == nil)
	if err != nil {
		conn.CloseNow()
		return e
	}
	principal, err := replica.RelayClient(ctx, conn, authority.Fingerprint)
	add("mutual_device_key_authentication", err == nil && relayPrincipalID(principal))
	if err != nil {
		conn.CloseNow()
		return e
	}
	defer conn.CloseNow()
	relayCapture(ctx, fixture, replica, authority.Fingerprint, add)
	add("hello_before_auth_rejected", relayRejectHello(ctx, fixture))
	add("binary_auth_frame_rejected", relayRejectBinary(ctx, fixture, replica))
	wrongPin, err := relayAuthDial(ctx, fixture)
	add("wrong_pin_tls_connection", err == nil)
	if err == nil {
		_, err = replica.RelayClient(ctx, wrongPin, strings.Repeat("0", 64))
		wrongPin.CloseNow()
	}
	add("webpki_alone_does_not_authenticate_gateway", err != nil)
	stranger, err := trust.Open(filepath.Join(w.Dir, "unpaired-relay"))
	add("unpaired_device_identity", err == nil)
	if err == nil {
		var unknown *websocket.Conn
		unknown, err = relayAuthDial(ctx, fixture)
		add("unpaired_device_tls_connection", err == nil)
		if err == nil {
			_, err = stranger.RelayClient(ctx, unknown, authority.Fingerprint)
			unknown.CloseNow()
		}
	}
	add("unpaired_device_key_rejected", err != nil)
	_, response, err := websocket.Dial(ctx, strings.Replace(fixture.URL, "https:", "wss:", 1)+"/v1/feed", &websocket.DialOptions{HTTPClient: fixture.Client(), HTTPHeader: http.Header{"Origin": []string{"https://foreign.example.com"}}})
	add("browser_origin_rejected", err != nil && response != nil && response.StatusCode == 401)
	oldRevision := principal.Revision
	err = authority.ChangeScope(principal.ID, []contracts.Grant{"system:read"})
	add("scope_change_closes_authenticated_socket", err == nil && relayClosed(ctx, conn))
	scoped, err := relayAuthDial(ctx, fixture)
	if err == nil {
		principal, err = replica.RelayClient(ctx, scoped, authority.Fingerprint)
	}
	add("subscription_revision_and_scope_reauthenticated", err == nil && principal.Revision == oldRevision+1 && len(principal.Grants) == 1 && principal.Grants[0] == "system:read")
	if err == nil {
		defer scoped.CloseNow()
		started := time.Now()
		err = authority.Revoke(principal.ID)
		add("revocation_closes_within_one_second", err == nil && relayClosed(ctx, scoped) && time.Since(started) < time.Second)
		duration := float64(time.Since(started)) / float64(time.Millisecond)
		e.Latency["relay_revocation"] = Latency{Samples: 1, P50: duration, P95: duration, Max: duration}
	}
	blocked, err := relayAuthDial(ctx, fixture)
	if err == nil {
		_, err = replica.RelayClient(ctx, blocked, authority.Fingerprint)
		blocked.CloseNow()
	}
	add("revoked_device_key_rejected", err != nil)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
