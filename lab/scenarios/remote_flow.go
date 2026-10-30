package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/remote"
	"tailscale.com/ipn/ipnstate"
	"tailscale.com/tailcfg"
)

type remoteFlowOutput struct {
	mu   sync.Mutex
	data bytes.Buffer
}

func (o *remoteFlowOutput) Write(p []byte) (int, error) {
	o.mu.Lock()
	defer o.mu.Unlock()
	return o.data.Write(p)
}
func (o *remoteFlowOutput) String() string { o.mu.Lock(); defer o.mu.Unlock(); return o.data.String() }

type remoteFlowNode struct {
	stage    atomic.Int32
	listener net.Listener
	queries  atomic.Int32
	listens  atomic.Int32
	closed   atomic.Bool
}

func (n *remoteFlowNode) LocalClient() (remote.LocalClient, error) { return n, nil }
func (n *remoteFlowNode) Close() error                             { n.closed.Store(true); return n.listener.Close() }
func (n *remoteFlowNode) ListenFunnel(string, string) (net.Listener, error) {
	n.listens.Add(1)
	return n.listener, nil
}
func (n *remoteFlowNode) StatusWithoutPeers(context.Context) (*ipnstate.Status, error) {
	if n.stage.Load() == 0 {
		return &ipnstate.Status{BackendState: "NeedsLogin", AuthURL: "https://login.example.com/device/auth-secret-token"}, nil
	}
	caps := tailcfg.NodeCapMap{}
	if n.stage.Load() >= 2 {
		caps = tailcfg.NodeCapMap{tailcfg.CapabilityHTTPS: nil, tailcfg.NodeAttrFunnel: nil, tailcfg.NodeCapability("https://tailscale.com/cap/funnel-ports?ports=443"): nil}
	}
	return &ipnstate.Status{BackendState: "Running", Self: &ipnstate.PeerStatus{DNSName: "bridge.example.com.", CapMap: caps}}, nil
}
func (n *remoteFlowNode) QueryFeature(_ context.Context, feature string) (*tailcfg.QueryFeatureResponse, error) {
	if feature != "funnel" {
		return nil, fmt.Errorf("unexpected feature")
	}
	n.queries.Add(1)
	if n.stage.Load() == -1 {
		return nil, fmt.Errorf("failed https://login.example.com/device/auth-secret-token")
	}
	if n.stage.Load() < 2 {
		return &tailcfg.QueryFeatureResponse{URL: "https://login.example.com/funnel?token=feature-secret-token", Text: "Enable Funnel https://login.example.com/funnel?token=feature-secret-token"}, nil
	}
	return &tailcfg.QueryFeatureResponse{Complete: true}, nil
}

func runRemoteFlow(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), Scenario: s.ID, Level: level, RunID: "remote-flow", At: time.Now().UTC(), NetworkPath: "isolated fake LocalClient and TCP listener; no tailscaled or login", Latency: map[string]Latency{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	if level == "real" {
		return runRemotePrivateReal(ctx, s, level)
	}
	remoteProbeRejections(ctx, add)
	remoteCancelFlow(ctx, add)
	remoteFailureFlow(ctx, add)
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	home, err := os.MkdirTemp("", "bridge-remote-flow-")
	add("private_home", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(home)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	add("isolated_listener", err == nil)
	if err != nil {
		return e
	}
	defer listener.Close()
	node := &remoteFlowNode{listener: listener}
	var output remoteFlowOutput
	var snippets bytes.Buffer
	opened := []string{}
	states := []string{}
	readStatus := func() remote.Status {
		var data bytes.Buffer
		remote.Command(ctx, home, "status", "", "", &data)
		var st remote.Status
		json.Unmarshal(data.Bytes(), &st)
		return st
	}
	transport := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", listener.Addr().String())
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport}
	options := remote.Options{Node: func(string, string) (remote.EmbeddedNode, error) { return node, nil }, Recheck: 10 * time.Millisecond, Probe: func(ctx context.Context, origin string) (time.Time, error) {
		st := readStatus()
		states = append(states, st.State)
		add("url_not_printed_before_probe", !strings.Contains(output.String(), origin))
		_, err := remote.ProbeHTTP(ctx, client, "http://bridge.example.com", 60*time.Millisecond)
		return time.Date(2030, 1, 1, 0, 0, 0, 0, time.UTC), err
	}}
	r := remote.NewWithOptions(home, options)
	web := http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) { w.WriteHeader(200) })
	api := http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		w.Header().Set("WWW-Authenticate", `Bearer resource_metadata="https://bridge.example.com/.well-known/oauth-protected-resource"`)
		w.WriteHeader(401)
	})
	done := make(chan struct{})
	go func() { defer close(done); r.Run(ctx, web, api, nil) }()
	defer func() { cancel(); <-done }()
	opener := func(ctx context.Context, link string) error {
		opened = append(opened, link)
		states = append(states, readStatus().State)
		remoteFilesPrivate(home, add)
		if link == "https://login.example.com/device/auth-secret-token" {
			node.stage.Store(1)
		} else if link == "https://login.example.com/funnel?token=feature-secret-token" {
			node.stage.Store(2)
		} else {
			return fmt.Errorf("unexpected link")
		}
		return nil
	}
	err = remote.CommandWithOpener(ctx, home+string(os.PathSeparator), "on", "", "", &output, opener)
	add("on_completed", err == nil)
	st := readStatus()
	add("embedded_default_and_ready", st.State == "on" && st.Backend == "embedded")
	add("approval_links_from_fake", strings.Join(opened, ",") == "https://login.example.com/device/auth-secret-token,https://login.example.com/funnel?token=feature-secret-token")
	add("states_walk", strings.Join(states, ",") == "login_pending,funnel_pending,probing")
	add("funnel_rechecked", node.queries.Load() >= 2)
	add("url_and_mcp_printed", strings.HasSuffix(output.String(), "https://bridge.example.com\nhttps://bridge.example.com/mcp\n"))
	add("status_fields", st.PendingLink == "" && st.TLSTermination == "on this Mac" && !st.CertificateExpiry.IsZero() && st.LastError == "")
	add("snippets", remote.Snippets(home, &snippets) == nil && strings.Contains(snippets.String(), "claude mcp add --transport http apple-bridge https://bridge.example.com/mcp") && strings.Contains(snippets.String(), "[mcp_servers.apple-bridge]") && strings.Contains(snippets.String(), "Grok"))
	stream, streamErr := client.Get("http://bridge.example.com/.well-known/apple-bridge-remote-check")
	add("active_stream_before_off", streamErr == nil)
	streamDone := make(chan struct{})
	if streamErr == nil {
		go func() { io.Copy(io.Discard, stream.Body); stream.Body.Close(); close(streamDone) }()
	} else {
		close(streamDone)
	}
	start := time.Now()
	err = remote.Command(ctx, home, "off", "", "", &bytes.Buffer{})
	elapsed := float64(time.Since(start)) / float64(time.Millisecond)
	e.Latency["remote_off"] = Latency{Samples: 1, P50: elapsed, P95: elapsed, Max: elapsed}
	add("off_within_one_second", err == nil && elapsed <= 1000)
	select {
	case <-streamDone:
		add("off_closes_active_stream", true)
	case <-time.After(time.Second):
		add("off_closes_active_stream", false)
	}
	response, err := client.Get("http://bridge.example.com/")
	if response != nil {
		response.Body.Close()
	}
	add("serving_stopped", err != nil)
	add("off_status", readStatus().State == "off")
	e.Passed = true
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func remoteProbeRejections(ctx context.Context, add func(string, bool)) {
	for _, failure := range []string{"discovery", "challenge", "web", "stream"} {
		server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			switch r.URL.Path {
			case "/.well-known/oauth-authorization-server":
				if failure == "discovery" {
					w.WriteHeader(302)
				}
			case "/mcp":
				if failure != "challenge" {
					w.Header().Set("WWW-Authenticate", "Bearer")
				}
				w.WriteHeader(401)
			case "/":
				if failure == "web" {
					w.WriteHeader(503)
				}
			default:
				w.Header().Set("Content-Type", "text/event-stream")
				fmt.Fprint(w, ": bridge remote check\n\n")
			}
		}))
		_, err := remote.ProbeHTTP(ctx, server.Client(), server.URL, time.Second)
		add("probe_rejects_"+failure, err != nil)
		server.Close()
	}
}

func remoteFilesPrivate(home string, add func(string, bool)) {
	safe := true
	filepath.WalkDir(home, func(path string, entry os.DirEntry, err error) error {
		if err != nil {
			safe = false
			return nil
		}
		if entry.Type().IsRegular() {
			data, err := os.ReadFile(path)
			safe = safe && err == nil && !bytes.Contains(data, []byte("auth-secret-token")) && !bytes.Contains(data, []byte("feature-secret-token"))
		}
		return nil
	})
	add("approval_tokens_absent_from_status_and_logs", safe)
}

func remoteCancelFlow(ctx context.Context, add func(string, bool)) {
	home, err := os.MkdirTemp("", "rmt-cancel-")
	if err != nil {
		add("cancel_home", false)
		return
	}
	defer os.RemoveAll(home)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		add("cancel_listener", false)
		return
	}
	defer listener.Close()
	node := &remoteFlowNode{listener: listener}
	node.stage.Store(1)
	runCtx, stop := context.WithCancel(ctx)
	done := make(chan struct{})
	r := remote.NewWithOptions(home, remote.Options{Node: func(string, string) (remote.EmbeddedNode, error) { return node, nil }, Recheck: 10 * time.Millisecond, Probe: func(context.Context, string) (time.Time, error) { return time.Now(), nil }})
	go func() { defer close(done); r.Run(runCtx, http.NotFoundHandler(), nil, nil) }()
	defer func() { stop(); <-done }()
	waitCtx, cancel := context.WithCancel(ctx)
	pending := make(chan struct{}, 1)
	result := make(chan error, 1)
	go func() {
		result <- remote.CommandWithOpener(waitCtx, home, "on", "", "", io.Discard, func(context.Context, string) error {
			select {
			case pending <- struct{}{}:
			default:
			}
			return nil
		})
	}()
	select {
	case <-pending:
	case <-time.After(3 * time.Second):
		add("cancel_reached_pending", false)
		cancel()
		return
	}
	remoteFilesPrivate(home, add)
	cancel()
	select {
	case err := <-result:
		add("cancel_returns", err != nil)
	case <-time.After(time.Second):
		add("cancel_returns", false)
	}
	deadline := time.Now().Add(time.Second)
	for !node.closed.Load() && time.Now().Before(deadline) {
		time.Sleep(10 * time.Millisecond)
	}
	queries := node.queries.Load()
	node.stage.Store(2)
	time.Sleep(350 * time.Millisecond)
	var status bytes.Buffer
	remote.Command(ctx, home, "status", "", "", &status)
	var st remote.Status
	json.Unmarshal(status.Bytes(), &st)
	conn, dialErr := net.DialTimeout("tcp", listener.Addr().String(), 100*time.Millisecond)
	if conn != nil {
		conn.Close()
	}
	add("cancel_mid_wait_never_serves_after_funnel_enabled", st.State == "off" && dialErr != nil && node.closed.Load() && node.listens.Load() == 0 && node.queries.Load() == queries)
	remoteFilesPrivate(home, add)
}

func remoteFailureFlow(ctx context.Context, add func(string, bool)) {
	home, err := os.MkdirTemp("", "rmt-error-")
	if err != nil {
		add("error_home", false)
		return
	}
	defer os.RemoveAll(home)
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		add("error_listener", false)
		return
	}
	defer listener.Close()
	node := &remoteFlowNode{listener: listener}
	node.stage.Store(-1)
	runCtx, stop := context.WithCancel(ctx)
	done := make(chan struct{})
	r := remote.NewWithOptions(home, remote.Options{Node: func(string, string) (remote.EmbeddedNode, error) { return node, nil }})
	go func() { defer close(done); r.Run(runCtx, http.NotFoundHandler(), nil, nil) }()
	defer func() { stop(); <-done }()
	bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	err = remote.CommandWithOpener(bounded, home, "on", "", "", io.Discard, func(context.Context, string) error { return nil })
	add("client_error_redacted", err != nil && !strings.Contains(err.Error(), "auth-secret-token") && bounded.Err() == nil)
	remoteFilesPrivate(home, add)
}
