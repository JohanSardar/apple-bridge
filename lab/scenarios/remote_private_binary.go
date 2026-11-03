package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/phonefixture"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func runRustPrivateConnect(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: "private-localapi", NetworkPath: "Rust binary through isolated Tailscale app LocalAPI socket"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	dir, err := os.MkdirTemp("", "rmt-localapi-")
	if err != nil {
		add("home", false)
		return e
	}
	defer os.RemoveAll(dir)
	binary, err := world.ImplementationSelection(ctx).Binary("rust", "")
	if err != nil {
		add("binary", false)
		return e
	}
	gateway, err := phonefixture.StartPrivate(ctx, binary, filepath.Join(dir, "home"))
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "private_on", Detail: err.Error()})
		return e
	}
	defer gateway.Close()
	statusOutput, statusErr := gateway.Command("remote", "status")
	add("status_cli", statusErr == nil)
	raw := []byte(statusOutput)
	var status struct{ State, Backend, Message string }
	json.Unmarshal(raw, &status)
	add("status_on_private", status.State == "on" && status.Backend == "private")
	code := strings.TrimPrefix(status.Message, "bridge connect ")
	decoded, _ := base64.RawURLEncoding.DecodeString(code)
	var invite struct{ Host, Secret string }
	err = json.Unmarshal(decoded, &invite)
	add("private_invitation_valid", err == nil && invite.Host == "node.fixture.ts.net" && len(invite.Secret) == 43)
	var lease struct{ Target string }
	raw, _ = os.ReadFile(filepath.Join(gateway.Home, "private/remote/lease.json"))
	json.Unmarshal(raw, &lease)
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", lease.Target)
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 3 * time.Second}
	response, err := client.Get("http://localhost:7780/mcp")
	add("session_required", err == nil && response.StatusCode == 403)
	if response != nil {
		response.Body.Close()
	}
	listener, _ := net.Listen("tcp", "127.0.0.1:0")
	address := listener.Addr().String()
	listener.Close()
	connectCtx, cancel := context.WithCancel(ctx)
	defer cancel()
	cmd := exec.CommandContext(connectCtx, binary, "connect", code, "--home", filepath.Join(dir, "connect"), "--listen", address)
	cmd.Env = append(os.Environ(), "APPLE_BRIDGE_TAILSCALE_SOCKET="+filepath.Join(dir, "tailscale.sock"))
	var output remoteFlowOutput
	cmd.Stdout = &output
	cmd.Stderr = &output
	err = cmd.Start()
	add("connect_started", err == nil)
	if err != nil {
		return e
	}
	defer func() { cancel(); cmd.Wait() }()
	for deadline := time.Now().Add(5 * time.Second); !strings.Contains(output.String(), "http://localhost:7780") && time.Now().Before(deadline); {
		time.Sleep(10 * time.Millisecond)
	}
	add("invitation_redeemed", strings.Contains(output.String(), "http://localhost:7780"))
	localTransport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", address)
	}}
	defer localTransport.CloseIdleConnections()
	local := &http.Client{Transport: localTransport, Timeout: 3 * time.Second}
	response, err = local.Get("http://localhost:7780/mcp")
	add("session_reaches_bridge_challenge", err == nil && response.StatusCode == 401 && strings.Contains(response.Header.Get("WWW-Authenticate"), "Bearer"))
	if response != nil {
		response.Body.Close()
	}
	request, _ := http.NewRequest("POST", "http://localhost:7780/_bridge/private/invite", nil)
	request.Header.Set("Authorization", "Bearer "+invite.Secret)
	response, err = client.Do(request)
	add("invitation_reuse_rejected", err == nil && response.StatusCode == 403)
	if response != nil {
		response.Body.Close()
	}
	token, err := gateway.Command("token", "create", "--scope", "system:read")
	add("api_token", err == nil)
	authenticated := &http.Client{Transport: privateTokenTransport{localTransport, token}, Timeout: 3 * time.Second}
	privateBinaryMCP(ctx, authenticated, add)
	response, err = authenticated.Get("http://localhost:7780/v1/feed")
	body := ""
	if err == nil {
		data, _ := io.ReadAll(response.Body)
		body = string(data)
	}
	add("private_feed_dispatch", err == nil && response.StatusCode == 200 && strings.Contains(body, "private-feed"))
	if response != nil {
		response.Body.Close()
	}
	response, err = local.Get("http://localhost:7780/")
	add("private_transport_retains_authentication", err == nil && response.StatusCode == 401)
	if response != nil {
		response.Body.Close()
	}
	safe := true
	filepath.WalkDir(filepath.Join(dir, "connect"), func(path string, d os.DirEntry, err error) error {
		if err == nil && !d.IsDir() {
			data, _ := os.ReadFile(path)
			safe = safe && !strings.Contains(string(data), "tailscaled.state") && !strings.Contains(d.Name(), "tailscaled.state")
		}
		return nil
	})
	add("connect_has_no_persisted_identity", safe)

	expiredDir := filepath.Join(dir, "expired")
	os.MkdirAll(expiredDir, 0700)
	expired, expiryErr := phonefixture.StartPrivateExpired(ctx, binary, filepath.Join(expiredDir, "home"))
	if expiryErr != nil {
		add("expired_invitation_rejected", false)
	} else {
		defer expired.Close()
		raw, _ := os.ReadFile(filepath.Join(expired.Home, "remote/status.json"))
		json.Unmarshal(raw, &status)
		expiredCmd := exec.CommandContext(ctx, binary, "connect", strings.TrimPrefix(status.Message, "bridge connect "), "--home", filepath.Join(expiredDir, "connect"), "--listen", "127.0.0.1:0")
		expiredCmd.Env = append(os.Environ(), "APPLE_BRIDGE_TAILSCALE_SOCKET="+filepath.Join(expiredDir, "tailscale.sock"))
		out, expiryErr := expiredCmd.CombinedOutput()
		add("expired_invitation_rejected", expiryErr != nil && strings.Contains(string(out), "private invitation rejected"))
	}
	remoteFilesPrivate(gateway.Home, add)
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func privateBinaryMCP(ctx context.Context, client *http.Client, add func(string, bool)) {
	session, err := mcp.NewClient(&mcp.Implementation{Name: "rust-private", Version: "1"}, nil).Connect(ctx, &mcp.StreamableClientTransport{Endpoint: "http://localhost:7780/mcp", HTTPClient: client}, nil)
	ok := err == nil
	if ok {
		_, err = session.ListTools(ctx, nil)
		ok = err == nil
		session.Close()
	}
	add("private_mcp_tools_list", ok)
}
