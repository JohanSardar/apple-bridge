package scenarios

import (
	"bytes"
	"context"
	"crypto/rand"
	"crypto/rsa"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"math/big"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sync/atomic"
	"time"
)

func runAgentOAuthCIMD(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	status, raw, _ := agentRequest(ctx, c.origin, "", "GET", "/.well-known/oauth-authorization-server", nil)
	add("metadata_documents_advertised", status == 200 && bytes.Contains(raw, []byte(`"client_id_metadata_document_supported":true`)))
	var hits atomic.Int64
	var privateHits atomic.Int64
	var privateConnections atomic.Int64
	metadata := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
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

	key, err := rsa.GenerateKey(rand.Reader, 2048)
	if err != nil {
		add("fixture_key", false)
		return
	}
	cert := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "CIMD lab"}, DNSNames: []string{"client.example", "private.example"}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, IsCA: true, BasicConstraintsValid: true}
	der, err := x509.CreateCertificate(rand.Reader, cert, cert, &key.PublicKey, key)
	if err != nil {
		add("fixture_certificate", false)
		return
	}
	certPEM := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	leaf := &x509.Certificate{SerialNumber: big.NewInt(2), Subject: pkix.Name{CommonName: "client.example"}, DNSNames: []string{"client.example", "private.example"}, NotBefore: cert.NotBefore, NotAfter: cert.NotAfter, KeyUsage: x509.KeyUsageDigitalSignature | x509.KeyUsageKeyEncipherment, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, BasicConstraintsValid: true}
	parent, err := x509.ParseCertificate(der)
	if err != nil {
		add("fixture_parent", false)
		return
	}
	leafDER, err := x509.CreateCertificate(rand.Reader, leaf, parent, &key.PublicKey, key)
	if err != nil {
		add("fixture_leaf", false)
		return
	}
	pair, err := tls.X509KeyPair(pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: leafDER}), pem.EncodeToMemory(&pem.Block{Type: "RSA PRIVATE KEY", Bytes: x509.MarshalPKCS1PrivateKey(key)}))
	if err != nil {
		add("fixture_tls", false)
		return
	}
	metadata.TLS = &tls.Config{Certificates: []tls.Certificate{pair}, MinVersion: tls.VersionTLS12}
	metadata.StartTLS()
	defer metadata.Close()
	var mu sync.Mutex
	ip := "8.8.8.8"
	rebinding := false
	resolutions := 0
	connections := 0
	now := time.Now().Unix()
	control := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		switch r.URL.Path {
		case "/resolve":
			resolutions++
			value := ip
			if r.URL.Query().Get("host") == "private.example" || rebinding && resolutions > 1 {
				value = "127.0.0.1"
			}
			json.NewEncoder(w).Encode([]string{value})
		case "/dial":
			connections++
			if strings.Contains(r.URL.Query().Get("host"), "private.example") || strings.Contains(r.URL.Query().Get("host"), "127.0.0.1") {
				privateConnections.Add(1)
			}
			json.NewEncoder(w).Encode(metadata.Listener.Addr().String())
		case "/clock":
			json.NewEncoder(w).Encode(now)
		}
	}))
	defer control.Close()
	ca := filepath.Join(w.Dir, "cimd-ca.pem")
	if os.WriteFile(ca, certPEM, 0600) != nil {
		add("fixture_ca", false)
		return
	}
	w.GatewayEnv = append(w.GatewayEnv, "APPLE_BRIDGE_LAB_CIMD="+control.URL, "APPLE_BRIDGE_LAB_CIMD_CA="+ca)
	if w.RestartGateway() != nil {
		add("fixture_restart", false)
		return
	}
	c, err = binaryOwnerSession(ctx, w, "trusted")
	if err != nil {
		add("fixture_owner", false)
		return
	}
	verifier := strings.Repeat("a", 43)
	hash := sha256.Sum256([]byte(verifier))
	q := url.Values{"client_id": {"https://client.example/good"}, "response_type": {"code"}, "redirect_uri": {"https://connector.example/callback"}, "scope": {"messages:read"}, "resource": {c.origin + "/mcp"}, "code_challenge": {base64.RawURLEncoding.EncodeToString(hash[:])}, "code_challenge_method": {"S256"}, "format": {"json"}}
	request := func(method string) (int, []byte) {
		select {
		case <-ctx.Done():
			return 0, nil
		case <-time.After(110 * time.Millisecond):
		}
		req, _ := http.NewRequestWithContext(ctx, method, c.origin+"/oauth/authorize?"+q.Encode(), strings.NewReader(`{"decision":"approve","level":"read","scopes":["messages:read"]}`))
		req.AddCookie(c.cookie)
		req.Header.Set("Origin", c.origin)
		req.Header.Set("Content-Type", "application/json")
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0, nil
		}
		defer resp.Body.Close()
		raw, _ := io.ReadAll(resp.Body)
		return resp.StatusCode, raw
	}
	fetch := func(id, redirect string) int {
		q.Set("client_id", id)
		q.Set("redirect_uri", redirect)
		code, _ := request("GET")
		return code
	}
	for _, id := range []string{"https://client.example/client#", "https://client.example/client#fragment", "https://client.example", "https://client.example/", "https://client.example/a/./client", "https://client.example/a/../client", "https://client.example/a/%2e/client", "https://client.example/a/%2E%2e/client"} {
		before := hits.Load()
		code := fetch(id, "https://connector.example/callback")
		add("invalid_client_id_"+id, code == 400 && hits.Load() == before)
	}
	for _, variant := range []string{"malformed-redirect", "unsupported-redirect", "mixed-redirects"} {
		redirect := "%"
		if variant == "unsupported-redirect" {
			redirect = "ftp://connector.example/callback"
		}
		if variant == "mixed-redirects" {
			redirect = "https://connector.example/callback"
		}
		code := fetch("https://client.example/"+variant, redirect)
		before := hits.Load()
		again := fetch("https://client.example/"+variant, redirect)
		add("invalid_document_"+variant, code == 400 && again == 400 && hits.Load() == before+1)
	}
	for _, ipValue := range []string{"0.1.2.3", "127.0.0.1", "10.0.0.1", "172.16.0.1", "192.168.0.1", "169.254.1.1", "100.64.0.1", "::1", "fc00::1", "fe80::1", "::ffff:127.0.0.1"} {
		mu.Lock()
		ip = ipValue
		mu.Unlock()
		before := hits.Load()
		code := fetch("https://client.example/good", "https://connector.example/callback")
		add("blocked_ip_"+ipValue, code == 400 && hits.Load() == before)
	}
	mu.Lock()
	ip = "8.8.8.8"
	rebinding = true
	resolutions = 0
	connections = 0
	mu.Unlock()
	code := fetch("https://client.example/rebinding", "https://connector.example/callback")
	mu.Lock()
	add("dns_rebinding_refused_before_connection", code == 400 && resolutions == 2 && connections == 0)
	rebinding = false
	mu.Unlock()
	code = fetch("https://client.example/cached", "https://connector.example/callback")
	before := hits.Load()
	cached := fetch("https://client.example/cached", "https://connector.example/callback")
	add("cache_control_reuses_document", code == 200 && cached == 200 && hits.Load() == before)
	mu.Lock()
	now += 61
	mu.Unlock()
	cached = fetch("https://client.example/cached", "https://connector.example/callback")
	add("cache_control_expires_document", cached == 200 && hits.Load() == before+1)
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
	status, raw, _ = agentRequest(ctx, c.origin, "", "POST", "/oauth/token", url.Values{"grant_type": {"authorization_code"}, "client_id": {q.Get("client_id")}, "code": {dest.Query().Get("code")}, "redirect_uri": {q.Get("redirect_uri")}, "code_verifier": {verifier}, "resource": {c.origin + "/mcp"}})
	var token oauthTokensFixture
	json.Unmarshal(raw, &token)
	add("metadata_token_issued", status == 200 && token.AccessToken != "")
	select {
	case <-ctx.Done():
		return
	case <-time.After(time.Second):
	}
	runAgentOAuth(ctx, w, c, e, add)
}
