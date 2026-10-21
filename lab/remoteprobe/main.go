package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/url"
	"os"
	"slices"
	"strings"
	"sync/atomic"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"
)

type input struct {
	URL    string
	IP     string
	Remote string
	Local  string
	Off    bool
}

type transport struct {
	base  http.RoundTripper
	token string
	sse   atomic.Bool
}

func (t *transport) RoundTrip(req *http.Request) (*http.Response, error) {
	req = req.Clone(req.Context())
	req.Header.Set("Authorization", "Bearer "+t.token)
	resp, err := t.base.RoundTrip(req)
	if err == nil && strings.HasPrefix(resp.Header.Get("Content-Type"), "text/event-stream") {
		t.sse.Store(true)
	}
	return resp, err
}

func main() {
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	var in input
	if json.NewDecoder(os.Stdin).Decode(&in) != nil {
		os.Exit(2)
	}
	u, err := url.Parse(in.URL)
	if err != nil {
		os.Exit(2)
	}
	port := u.Port()
	if port == "" {
		port = "443"
	}
	base := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, network, net.JoinHostPort(in.IP, port))
	}}
	defer base.CloseIdleConnections()
	auth := &transport{base: base, token: in.Remote}
	client := &http.Client{Transport: auth, Timeout: 10 * time.Second}
	checks := map[string]bool{}
	request := func(method, path, token, origin string) (int, []byte, *http.Response) {
		req, _ := http.NewRequestWithContext(ctx, method, in.URL+path, strings.NewReader("{\"limit\":1}"))
		if token != "" {
			req.Header.Set("Authorization", "Bearer "+token)
		}
		if origin != "" {
			req.Header.Set("Origin", origin)
			req.Header.Set("X-Forwarded-Host", "spoof.invalid")
			req.Header.Set("X-Forwarded-Proto", "http")
		}
		resp, err := (&http.Client{Transport: base, Timeout: 5 * time.Second}).Do(req)
		if err != nil {
			return 0, nil, nil
		}
		defer resp.Body.Close()
		data, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, data, resp
	}
	if in.Off {
		status, _, _ := request("GET", "/v1/health", "", "")
		checks["off_public_stopped"] = status != 200
		json.NewEncoder(os.Stdout).Encode(map[string]any{"checks": checks})
		return
	}
	status, body, resp := request("GET", "/", "", "")
	checks["html_valid_certificate"] = status == 200 && bytes.Contains(body, []byte(`id="root"`)) && resp.TLS != nil && len(resp.TLS.VerifiedChains) != 0
	status, _, _ = request("GET", "/v1/health", "", "")
	checks["public_health"] = status == 200
	status, _, _ = request("POST", "/v1/op/records.query", in.Remote, "")
	checks["remote_token_rest"] = status == 200
	status, _, _ = request("POST", "/v1/op/records.query", in.Local, "")
	checks["local_token_rejected"] = status == 401
	status, _, _ = request("POST", "/v1/op/records.query", in.Remote, in.URL)
	checks["origin_rejected"] = status == 401
	status, _, _ = request("GET", "/v1/feed", in.Remote, "")
	checks["feed_requires_replica_handshake"] = status == 400 || status == 401 || status == 426
	status, body, _ = request("POST", "/web/v1/login/options", "", in.URL)
	var options struct {
		PublicKey struct {
			RPID string `json:"rpId"`
		} `json:"publicKey"`
	}
	json.Unmarshal(body, &options)
	checks["forwarded_headers_ignored"] = status == 200
	checks["public_passkey_origin_rpid"] = status == 200 && options.PublicKey.RPID == u.Hostname()
	session, err := mcp.NewClient(&mcp.Implementation{Name: "bridge-funnel-lab", Version: "1"}, nil).Connect(ctx, &mcp.StreamableClientTransport{Endpoint: in.URL + "/mcp", HTTPClient: client}, nil)
	checks["mcp_initialize"] = err == nil
	if err == nil {
		tools, err := session.ListTools(ctx, &mcp.ListToolsParams{})
		checks["mcp_tools_list"] = err == nil && len(tools.Tools) > 0
		result, err := session.CallTool(ctx, &mcp.CallToolParams{Name: "records_query", Arguments: map[string]any{"limit": 1}})
		checks["mcp_read_tool"] = err == nil && !result.IsError
		checks["mcp_sse_response"] = auth.sse.Load()
		session.Close()
	}
	values := []float64{}
	for i := 0; i < 20; i++ {
		start := time.Now()
		status, _, _ := request("POST", "/v1/op/records.query", in.Remote, "")
		checks[fmt.Sprintf("read_%02d", i)] = status == 200
		values = append(values, float64(time.Since(start))/float64(time.Millisecond))
	}
	slices.Sort(values)
	json.NewEncoder(os.Stdout).Encode(map[string]any{"checks": checks, "latency": map[string]any{"samples": len(values), "p50_ms": values[9], "p95_ms": values[18], "max_ms": values[19]}})
}
