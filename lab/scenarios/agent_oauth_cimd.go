package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"sync/atomic"
	"time"
)

func runAgentOAuthCIMD(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	status, raw, _ := agentRequest(ctx, c.origin, "", "GET", "/.well-known/oauth-authorization-server", nil)
	add("metadata_documents_advertised", status == 200 && bytes.Contains(raw, []byte(`"client_id_metadata_document_supported":true`)))
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("authority", false)
		return
	}
	var hits atomic.Int64
	var privateHits atomic.Int64
	var privateConnections atomic.Int64
	metadata := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hits.Add(1)
		if r.Host == "private.example" {
			privateHits.Add(1)
		}
		if r.URL.Path == "/slow" {
			select {
			case <-r.Context().Done():
				return
			case <-time.After(4 * time.Second):
			}
		}
		if r.URL.Path == "/private-redirect" {
			http.Redirect(w, r, "https://private.example/client", 302)
			return
		}
		id := "https://client.example" + r.URL.Path
		if r.Host == "private.example" {
			id = "https://client.example/private-redirect"
		}
		if r.URL.Path == "/mismatch" {
			id = "https://other.example/client"
		}
		w.Header().Set("Cache-Control", "max-age=60")
		redirects := []string{"https://connector.example/callback"}
		if r.URL.Path == "/malformed-redirect" {
			redirects = []string{"%"}
		}
		if r.URL.Path == "/unsupported-redirect" {
			redirects = []string{"ftp://connector.example/callback"}
		}
		if r.URL.Path == "/mixed-redirects" {
			redirects = append(redirects, "%")
		}
		if r.URL.Path == "/oversize" {
			io.WriteString(w, strings.Repeat(" ", 5121))
		}
		json.NewEncoder(w).Encode(map[string]any{"client_id": id, "client_name": "Bridge Test metadata", "redirect_uris": redirects})
	}))
	defer metadata.Close()
	transport := metadata.Client().Transport.(*http.Transport).Clone()
	transport.TLSClientConfig.ServerName = "127.0.0.1"
	transport.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		host, _, _ := net.SplitHostPort(address)
		if host == "private.example" {
			privateConnections.Add(1)
		}
		return (&net.Dialer{}).DialContext(ctx, network, metadata.Listener.Addr().String())
	}
	defer transport.CloseIdleConnections()
	fetcher := &bridgeweb.ClientMetadataFetcher{Transport: transport, Resolve: func(ctx context.Context, host string) ([]net.IPAddr, error) {
		ip := "8.8.8.8"
		if host == "private.example" {
			ip = "127.0.0.1"
		}
		return []net.IPAddr{{IP: net.ParseIP(ip)}}, nil
	}}
	for _, id := range []string{"https://client.example/client#", "https://client.example/client#fragment", "https://client.example", "https://client.example/", "https://client.example/a/./client", "https://client.example/a/../client", "https://client.example/a/%2e/client", "https://client.example/a/%2E%2e/client"} {
		before := hits.Load()
		_, err := fetcher.Fetch(ctx, id, "https://connector.example/callback")
		add("invalid_client_id_"+id, err != nil && hits.Load() == before)
	}
	for _, variant := range []string{"malformed-redirect", "unsupported-redirect", "mixed-redirects"} {
		redirect := "%"
		if variant == "unsupported-redirect" {
			redirect = "ftp://connector.example/callback"
		}
		if variant == "mixed-redirects" {
			redirect = "https://connector.example/callback"
		}
		_, err := fetcher.Fetch(ctx, "https://client.example/"+variant, redirect)
		before := hits.Load()
		_, again := fetcher.Fetch(ctx, "https://client.example/"+variant, redirect)
		add("invalid_document_"+variant, err != nil && again != nil && hits.Load() == before+1)
	}
	for _, ip := range []string{"0.1.2.3", "127.0.0.1", "10.0.0.1", "172.16.0.1", "192.168.0.1", "169.254.1.1", "100.64.0.1", "::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1"} {
		blocked := &bridgeweb.ClientMetadataFetcher{Transport: transport, Resolve: func(context.Context, string) ([]net.IPAddr, error) { return []net.IPAddr{{IP: net.ParseIP(ip)}}, nil }}
		before := hits.Load()
		_, err := blocked.Fetch(ctx, "https://client.example/good", "https://connector.example/callback")
		add("blocked_ip_"+ip, err != nil && hits.Load() == before)
	}
	var resolutions atomic.Int64
	var connections atomic.Int64
	rebinding := &bridgeweb.ClientMetadataFetcher{
		Resolve: func(context.Context, string) ([]net.IPAddr, error) {
			ip := "8.8.8.8"
			if resolutions.Add(1) > 1 {
				ip = "127.0.0.1"
			}
			return []net.IPAddr{{IP: net.ParseIP(ip)}}, nil
		},
		DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			connections.Add(1)
			client, server := net.Pipe()
			go func() {
				defer server.Close()
				buf := make([]byte, 4096)
				server.Read(buf)
			}()
			return client, nil
		},
	}
	_, err = rebinding.Fetch(ctx, "https://client.example/rebinding", "https://connector.example/callback")
	add("dns_rebinding_refused_before_connection", err != nil && resolutions.Load() == 2 && connections.Load() == 0)
	_, err = fetcher.Fetch(ctx, "https://client.example/cached", "https://connector.example/callback")
	before := hits.Load()
	_, cachedErr := fetcher.Fetch(ctx, "https://client.example/cached", "https://connector.example/callback")
	add("cache_control_reuses_document", err == nil && cachedErr == nil && hits.Load() == before)
	fixture := httptest.NewUnstartedServer(nil)
	fixture.Config.Handler = bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: fixture.Listener.Addr().String(), ClientMetadata: fetcher})
	fixture.Start()
	defer fixture.Close()
	verifier := strings.Repeat("a", 43)
	hash := sha256.Sum256([]byte(verifier))
	q := url.Values{"client_id": {"https://client.example/good"}, "response_type": {"code"}, "redirect_uri": {"https://connector.example/callback"}, "scope": {"messages:read"}, "resource": {fixture.URL + "/mcp"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(hash[:])}, "code_challenge_method": {"S256"}, "format": {"json"}}
	request := func(method string) (int, []byte) {
		req, _ := http.NewRequestWithContext(ctx, method, fixture.URL+"/oauth/authorize?"+q.Encode(), strings.NewReader(`{"decision":"approve","level":"read","scopes":["messages:read"]}`))
		req.AddCookie(c.cookie)
		req.Header.Set("Origin", fixture.URL)
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0, nil
		}
		defer resp.Body.Close()
		raw, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, raw
	}
	for _, variant := range []string{"good", "mismatch", "oversize", "slow", "private-redirect", "private", "bad-redirect", "malformed-redirect", "unsupported-redirect", "mixed-redirects"} {
		id := "https://client.example/" + variant
		if variant == "private" {
			id = "https://private.example/client"
		}
		q.Set("client_id", id)
		q.Set("redirect_uri", "https://connector.example/callback")
		if variant == "bad-redirect" {
			q.Set("redirect_uri", "https://attacker.example/callback")
		}
		if variant == "malformed-redirect" {
			q.Set("redirect_uri", "%")
		}
		if variant == "unsupported-redirect" {
			q.Set("redirect_uri", "ftp://connector.example/callback")
		}
		status, raw = request("GET")
		if variant == "private-redirect" {
			add("private_redirect_no_connection", privateHits.Load() == 0 && privateConnections.Load() == 0)
		}
		if variant == "good" {
			add("metadata_valid_consent", status == 200)
		} else {
			add("reject_"+variant, status == 400 && bytes.Contains(raw, []byte(`"error":"invalid_request"`)))
		}
	}
	q.Set("client_id", "https://client.example/good")
	q.Set("redirect_uri", "https://connector.example/callback")
	status, raw = request("POST")
	var consent struct {
		Redirect string `json:"redirect"`
	}
	json.Unmarshal(raw, &consent)
	dest, _ := url.Parse(consent.Redirect)
	add("metadata_owner_consent", status == 200 && dest != nil && dest.Query().Get("code") != "")
	if dest == nil {
		return
	}
	status, raw, _ = agentRequest(ctx, fixture.URL, "", "POST", "/oauth/token", url.Values{"grant_type": {"authorization_code"}, "client_id": {q.Get("client_id")}, "code": {dest.Query().Get("code")}, "redirect_uri": {q.Get("redirect_uri")}, "code_verifier": {verifier}, "resource": {fixture.URL + "/mcp"}})
	var token trust.OAuthTokens
	json.Unmarshal(raw, &token)
	add("metadata_token_issued", status == 200 && token.AccessToken != "")
	runAgentOAuth(ctx, w, c, e, add)
}
