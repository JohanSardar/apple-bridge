package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"net/http"
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

func runRemoteSecurity(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	e = Evidence{GitSHA: strings.TrimSpace(string(sha)), Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: contracts.NewID("security"), NetworkPath: "isolated Mac gateway adversarial HTTP and trust checks", Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	finish := func() Evidence {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
		return e
	}
	if level == "sim" {
		approval := runConnectorApproval(ctx, s, "sim")
		e.Assertions = append(e.Assertions, approval.Assertions...)
		securityRelayProofs(add)
		securityPublicHealth(add)
		securityRemoteFailure(ctx, add)
	}
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("isolated_gateway", err == nil)
	if err != nil {
		return finish()
	}
	defer w.Close()
	a, err := trust.Open(w.Gateway.Home)
	add("authority", err == nil)
	if err != nil {
		return finish()
	}
	origin := w.Gateway.URL
	if level == "real" {
		e.NetworkPath = "public HTTPS through ab-relay ciphertext tunnel to isolated Mac gateway"
		e.Hosts = []string{"air", "ab-relay"}
		defer func() {
			bounded, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			offErr := remote.Command(bounded, w.Gateway.Home, "off", "", "", &bytes.Buffer{})
			add("public_relay_test_credentials_revoked_and_tunnel_closed", securityRevokeRelay(bounded, w.Gateway.Home) && offErr == nil)
			e = finish()
		}()
		bounded, cancel := context.WithTimeout(ctx, 150*time.Second)
		err = remote.Command(bounded, w.Gateway.Home, "on", "relay", "bridge", &bytes.Buffer{})
		cancel()
		add("public_relay_gateway_ready", err == nil)
		if err != nil {
			return finish()
		}
		raw, readErr := os.ReadFile(filepath.Join(w.Gateway.Home, "remote", "status.json"))
		var status remote.Status
		json.Unmarshal(raw, &status)
		add("public_https_origin", readErr == nil && status.State == "on" && status.Backend == "relay")
		origin = status.URL
	}
	e.Metrics["protected_endpoint_count"] = securityEndpointSweep(ctx, origin, level == "real", add)
	securityOAuth(ctx, w, a, origin, &e, add)
	token, _, err := a.CreateDeviceSession("trusted")
	add("owner_session", err == nil)
	cookie := bridgeweb.LocalSessionCookie
	if level == "real" {
		cookie = bridgeweb.SessionCookie
	}
	c := &webClient{ctx: ctx, origin: origin, cookie: &http.Cookie{Name: cookie, Value: token}}
	securityOwnerRequests(ctx, c, add)
	if level == "sim" {
		securityTrustBoundaries(w, a, add)
	}
	safe := true
	secrets := []string{token}
	stored, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "credentials.json"))
	safe = safe && !bytes.Contains(stored, []byte(token))
	for _, name := range []string{"gateway.log", "helper.log"} {
		log, readErr := os.ReadFile(filepath.Join(w.Dir, name))
		if os.IsNotExist(readErr) {
			continue
		}
		safe = safe && readErr == nil
		for _, secret := range secrets {
			safe = safe && secret != "" && !bytes.Contains(log, []byte(secret))
		}
	}
	add("owner_token_absent_from_storage_and_logs", safe)
	evidence, err := json.Marshal(e)
	add("evidence_contains_only_security_results", err == nil && !bytes.Contains(evidence, []byte(token)))
	return finish()
}

func securityEndpointSweep(ctx context.Context, origin string, public bool, add func(string, bool)) int {
	paths := []struct{ method, path string }{
		{"GET", "/mcp"}, {"POST", "/mcp"}, {"GET", "/mcp/unknown"},
		{"GET", "/v1/openapi.json"}, {"GET", "/v1/records/messages.chat/test"},
		{"GET", "/v1/changes"}, {"GET", "/v1/blobs/unknown"}, {"POST", "/v1/blobs"}, {"GET", "/v1/unknown"}, {"GET", "/v1/snapshot"}, {"POST", "/v1/commands"},
		{"GET", "/web/v1/session"}, {"GET", "/web/v1/agents"}, {"GET", "/web/v1/devices"},
		{"GET", "/web/v1/passkeys"}, {"GET", "/web/v1/activity"}, {"GET", "/web/v1/approvals"},
		{"GET", "/web/v1/sources"}, {"GET", "/web/v1/live"}, {"POST", "/web/v1/handles"}, {"POST", "/web/v1/lookup"}, {"POST", "/web/v1/upload"}, {"GET", "/web/v1/blob/unknown"}, {"GET", "/web/v1/upload/unknown"},
		{"POST", "/oauth/authorize"}, {"POST", "/web/v1/approvals/test"},
	}
	if public {
		paths = append(paths, struct{ method, path string }{"GET", "/v1/health"})
	}
	for _, d := range contracts.Operations {
		paths = append(paths, struct{ method, path string }{"POST", "/v1/op/" + d.Name})
	}
	all := true
	for _, p := range paths {
		status, _, _ := agentRequest(ctx, origin, "", p.method, p.path, map[string]any{})
		denied := status == 401 || status == 403
		all = all && denied
		if !denied {
			add("unauthenticated_denied_"+p.method+"_"+p.path, false)
		}
	}
	add("all_protected_endpoints_refuse_unauthenticated_requests", all)
	for _, path := range []string{"/", "/.well-known/oauth-authorization-server", "/.well-known/oauth-protected-resource"} {
		status, _, _ := agentRequest(ctx, origin, "", "GET", path, nil)
		add("public_bootstrap_"+path, status == 200)
	}
	return len(paths)
}
