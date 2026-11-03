package fixture

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
)

type dns struct {
	mu     sync.Mutex
	value  string
	writes int
}

func (d *dns) TXT(_ context.Context, _, value string) error {
	d.mu.Lock()
	defer d.mu.Unlock()
	d.value = value
	d.writes++
	return nil
}
func save(t *testing.T, path string, value any) {
	t.Helper()
	b, e := json.Marshal(value)
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(path, b, 0600); e != nil {
		t.Fatal(e)
	}
}
func invoke(t *testing.T, args ...string) []byte {
	t.Helper()
	c := exec.Command(os.Getenv("AB_REMOTE_BINARY"), args...)
	b, e := c.CombinedOutput()
	if e != nil {
		t.Fatalf("driver %v: %v %s", args, e, b)
	}
	return b
}
func cert(t *testing.T, home, host string, days int) []byte {
	t.Helper()
	key, e := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if e != nil {
		t.Fatal(e)
	}
	template := &x509.Certificate{SerialNumber: big.NewInt(time.Now().UnixNano()), Subject: pkix.Name{CommonName: host}, DNSNames: []string{host}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Duration(days) * 24 * time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	if ip := net.ParseIP(host); ip != nil {
		template.IPAddresses = []net.IP{ip}
		template.DNSNames = nil
	}
	der, e := x509.CreateCertificate(rand.Reader, template, template, &key.PublicKey, key)
	if e != nil {
		t.Fatal(e)
	}
	raw := pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
	kd, e := x509.MarshalECPrivateKey(key)
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(filepath.Join(home, "remote/relay-cert.pem"), raw, 0600); e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(filepath.Join(home, "remote/relay-key.pem"), pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: kd}), 0600); e != nil {
		t.Fatal(e)
	}
	return raw
}

func TestConcurrentIngress(t *testing.T) {
	if os.Getenv("AB_REMOTE_BINARY") == "" {
		t.Skip("set AB_REMOTE_BINARY to the isolated Rust driver")
	}
	home := t.TempDir()
	if e := os.Mkdir(filepath.Join(home, "remote"), 0700); e != nil {
		t.Fatal(e)
	}
	pub, key, e := ed25519.GenerateKey(rand.Reader)
	if e != nil {
		t.Fatal(e)
	}
	owners := filepath.Join(t.TempDir(), "owners")
	if e = os.WriteFile(owners, []byte(base64.RawURLEncoding.EncodeToString(pub)), 0600); e != nil {
		t.Fatal(e)
	}
	store, e := relay.Open(filepath.Join(t.TempDir(), "store.json"), "fixture.test", relay.Limits{Connections: 32, BytesPerSecond: 100 << 20, MonthlyBytes: 1 << 30})
	if e != nil {
		t.Fatal(e)
	}
	store.RequireOwnerKeys(owners)
	server := relay.New(store)
	defer server.Close()
	dns := &dns{}
	server.DNS = dns
	controlHome := t.TempDir()
	os.Mkdir(filepath.Join(controlHome, "remote"), 0700)
	controlCert := cert(t, controlHome, "127.0.0.1", 90)
	controlKey, _ := os.ReadFile(filepath.Join(controlHome, "remote/relay-key.pem"))
	pair, e := tls.X509KeyPair(controlCert, controlKey)
	if e != nil {
		t.Fatal(e)
	}
	control := httptest.NewUnstartedServer(server.Handler(""))
	control.TLS = &tls.Config{Certificates: []tls.Certificate{pair}}
	control.StartTLS()
	defer control.Close()
	ca := filepath.Join(t.TempDir(), "ca.pem")
	if e = os.WriteFile(ca, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: control.Certificate().Raw}), 0600); e != nil {
		t.Fatal(e)
	}
	save(t, filepath.Join(home, "remote/relay.json"), map[string]any{"key": key, "credential": relay.Credential{}, "endpoint": control.URL})
	var credential relay.Credential
	if e = json.Unmarshal(invoke(t, "enroll", "--home", home, "--endpoint", control.URL, "--ca", ca), &credential); e != nil {
		t.Fatal(e)
	}
	if credential.Hostname == "" {
		t.Fatal("no enrollment")
	}
	denied := exec.Command(os.Getenv("AB_REMOTE_BINARY"), "enroll", "--home", t.TempDir(), "--endpoint", control.URL, "--ca", ca)
	if denied.Run() == nil {
		t.Fatal("owner gate bypass")
	}
	old := cert(t, home, credential.Hostname, 1)
	originalKey, e := os.ReadFile(filepath.Join(home, "remote/relay-key.pem"))
	if e != nil {
		t.Fatal(e)
	}
	acme := acmeFixture(t, credential.Hostname, dns, pair)
	defer acme.Close()
	invoke(t, "renew", "--home", home, "--directory", acme.URL+"/directory", "--ca", ca)
	renewed, e := os.ReadFile(filepath.Join(home, "remote/relay-cert.pem"))
	if e != nil || bytes.Equal(old, renewed) {
		t.Fatal("renewal did not replace certificate", e)
	}
	oldKey, e := os.ReadFile(filepath.Join(home, "remote/relay-key.pem"))
	if !bytes.Equal(originalKey, oldKey) {
		t.Fatal("renewal replaced the TLS key")
	}
	dns.mu.Lock()
	dnsOK := dns.writes >= 2 && dns.value == ""
	dns.mu.Unlock()
	if !dnsOK {
		t.Fatal("DNS challenge was not published and cleared")
	}
	if e != nil {
		t.Fatal(e)
	}
	invoke(t, "renew", "--home", home, "--directory", acme.URL+"/directory", "--ca", ca)
	newKey, _ := os.ReadFile(filepath.Join(home, "remote/relay-key.pem"))
	if !bytes.Equal(oldKey, newKey) {
		t.Fatal("renewal key changed")
	}
	for _, name := range []string{"relay.json", "acme-account.pem", "relay-key.pem", "relay-cert.pem"} {
		info, e := os.Stat(filepath.Join(home, "remote", name))
		if e != nil || info.Mode().Perm() != 0600 {
			t.Fatal("secret permissions", name, e)
		}
	}
	var mu sync.Mutex
	config := map[string]any{"TCP": map[string]any{"443": map[string]any{"HTTPS": true}}, "Web": map[string]any{"other.test:443": map[string]any{"Handlers": map[string]any{"/": map[string]any{"Text": "preserve"}}}}}
	etag := 1
	conflicted := false
	local := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		w.Header().Set("Content-Type", "application/json")
		if strings.HasPrefix(r.URL.Path, "/status") {
			json.NewEncoder(w).Encode(map[string]any{"BackendState": "Running", "Self": map[string]any{"DNSName": "node.fixture.test.", "CapMap": map[string]any{"https": nil, "funnel": nil, "https://tailscale.com/cap/funnel-ports?ports=443,8443,10000": nil}}})
			return
		}
		if r.Method == "GET" {
			w.Header().Set("ETag", fmt.Sprint(etag))
			json.NewEncoder(w).Encode(config)
			return
		}
		if r.Header.Get("If-Match") != fmt.Sprint(etag) {
			w.WriteHeader(412)
			return
		}
		if !conflicted {
			conflicted = true
			etag++
			config["unrelated"] = "concurrent edit"
			w.WriteHeader(412)
			return
		}
		if json.NewDecoder(r.Body).Decode(&config) != nil {
			w.WriteHeader(400)
			return
		}
		etag++
	}))
	defer local.Close()
	marker := strings.Repeat("gateway-", 100000)
	gateway := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { fmt.Fprint(w, marker) }))
	defer gateway.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	public, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		t.Fatal(e)
	}
	defer public.Close()
	go server.Serve(ctx, public)
	cmd := exec.Command(os.Getenv("AB_REMOTE_BINARY"), "serve", "--home", home, "--via", "both", "--endpoint", control.URL, "--ca", ca, "--directory", acme.URL+"/directory", "--gateway", strings.TrimPrefix(gateway.URL, "http://"), "--localapi", local.URL)
	var logs bytes.Buffer
	cmd.Stderr = &logs
	if e = cmd.Start(); e != nil {
		t.Fatal(e)
	}
	defer func() { cmd.Process.Kill(); cmd.Wait() }()
	var lease struct {
		Host   string
		Port   uint16
		Target string
	}
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		b, e := os.ReadFile(filepath.Join(home, "remote/lease.json"))
		if e == nil {
			json.Unmarshal(b, &lease)
			if lease.Port != 0 {
				break
			}
		}
		time.Sleep(20 * time.Millisecond)
	}
	if lease.Port != 8443 {
		t.Fatalf("port %d logs %s", lease.Port, logs.String())
	}
	pool := x509.NewCertPool()
	pool.AppendCertsFromPEM(renewed)
	client := &http.Client{Timeout: 5 * time.Second, Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: pool, ServerName: credential.Hostname}, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", public.Addr().String())
	}}}
	var body []byte
	for time.Now().Before(deadline) {
		resp, e := client.Get("https://" + credential.Hostname + "/marker")
		if e == nil {
			body, e = io.ReadAll(resp.Body)
			resp.Body.Close()
			if e == nil && string(body) == marker {
				break
			}
		}
		time.Sleep(50 * time.Millisecond)
	}
	if string(body) != marker {
		t.Fatalf("public TLS stream failed: %s", logs.String())
	}
	req, _ := http.NewRequest("GET", lease.Target+"/marker", nil)
	req.Host = "node.fixture.test:8443"
	req.Header.Set("X-Forwarded-Host", req.Host)
	req.Header.Set("X-Forwarded-Proto", "https")
	req.Header.Set("X-Forwarded-For", "192.0.2.1")
	resp, e := http.DefaultClient.Do(req)
	if e != nil {
		t.Fatal(e)
	}
	b, e := io.ReadAll(resp.Body)
	resp.Body.Close()
	if e != nil || string(b) != marker {
		t.Fatal("capability listener did not reach gateway", e)
	}
	mu.Lock()
	preserved := config["unrelated"] == "concurrent edit" && config["Web"].(map[string]any)["other.test:443"] != nil
	mu.Unlock()
	if !preserved {
		t.Fatal("unrelated mapping lost")
	}
	if e = cmd.Process.Signal(os.Interrupt); e != nil {
		t.Fatal(e)
	}
	if e = cmd.Wait(); e != nil {
		t.Fatalf("shutdown %v %s", e, logs.String())
	}
	mu.Lock()
	defer mu.Unlock()
	if config["Web"].(map[string]any)["node.fixture.test:8443"] != nil || config["Web"].(map[string]any)["other.test:443"] == nil {
		t.Fatal("mapping cleanup lost unrelated handler")
	}
}

func acmeFixture(t *testing.T, host string, dns *dns, pair tls.Certificate) *httptest.Server {
	t.Helper()
	var server *httptest.Server
	ready := false
	issued := false
	var chain []byte
	var mu sync.Mutex
	server = httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		defer mu.Unlock()
		base := server.URL
		w.Header().Set("Replay-Nonce", base64.RawURLEncoding.EncodeToString([]byte(fmt.Sprint(time.Now().UnixNano()))))
		w.Header().Set("Content-Type", "application/json")
		reply := func(value any) { json.NewEncoder(w).Encode(value) }
		switch r.URL.Path {
		case "/directory":
			reply(map[string]any{"newNonce": base + "/nonce", "newAccount": base + "/account", "newOrder": base + "/new-order"})
		case "/nonce":
			w.WriteHeader(200)
		case "/account":
			w.Header().Set("Location", base+"/account/1")
			w.WriteHeader(201)
			reply(map[string]any{"status": "valid", "orders": base + "/orders"})
		case "/new-order", "/order":
			w.Header().Set("Location", base+"/order")
			state := "pending"
			if ready {
				state = "ready"
			}
			if issued {
				state = "valid"
			}
			reply(map[string]any{"status": state, "identifiers": []any{map[string]any{"type": "dns", "value": host}}, "authorizations": []string{base + "/authz"}, "finalize": base + "/finalize", "certificate": base + "/cert"})
		case "/authz":
			state := "pending"
			if ready {
				state = "valid"
			}
			reply(map[string]any{"identifier": map[string]any{"type": "dns", "value": host}, "status": state, "challenges": []any{map[string]any{"type": "dns-01", "url": base + "/challenge", "status": state, "token": "fixture-dns-token"}}})
		case "/challenge":
			dns.mu.Lock()
			valid := dns.value != ""
			dns.mu.Unlock()
			if !valid {
				w.WriteHeader(400)
				return
			}
			ready = true
			reply(map[string]any{"type": "dns-01", "url": base + "/challenge", "status": "valid", "token": "fixture-dns-token"})
		case "/finalize":
			var jws struct{ Payload string }
			json.NewDecoder(r.Body).Decode(&jws)
			payload, e := base64.RawURLEncoding.DecodeString(jws.Payload)
			if e != nil {
				w.WriteHeader(400)
				return
			}
			var value struct{ CSR string }
			json.Unmarshal(payload, &value)
			raw, e := base64.RawURLEncoding.DecodeString(value.CSR)
			if e != nil {
				w.WriteHeader(400)
				return
			}
			csr, e := x509.ParseCertificateRequest(raw)
			if e != nil || csr.CheckSignature() != nil || len(csr.DNSNames) != 1 || csr.DNSNames[0] != host {
				w.WriteHeader(400)
				return
			}
			key, _ := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
			template := &x509.Certificate{SerialNumber: big.NewInt(1), DNSNames: csr.DNSNames, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(90 * 24 * time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
			der, e := x509.CreateCertificate(rand.Reader, template, template, csr.PublicKey, key)
			if e != nil {
				w.WriteHeader(500)
				return
			}
			chain = pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})
			issued = true
			reply(map[string]any{"status": "valid", "identifiers": []any{map[string]any{"type": "dns", "value": host}}, "authorizations": []string{base + "/authz"}, "finalize": base + "/finalize", "certificate": base + "/cert"})
		case "/cert":
			w.Header().Set("Content-Type", "application/pem-certificate-chain")
			w.Write(chain)
		default:
			w.WriteHeader(404)
		}
	}))
	server.TLS = &tls.Config{Certificates: []tls.Certificate{pair}}
	server.StartTLS()
	return server
}

func TestSelectionRace(t *testing.T) {
	if os.Getenv("AB_REMOTE_BINARY") == "" {
		t.Skip("set AB_REMOTE_BINARY to the isolated Rust driver")
	}
	home := t.TempDir()
	commands := []*exec.Cmd{exec.Command(os.Getenv("AB_REMOTE_BINARY"), "remote", "on", "--via", "relay", "--home", home), exec.Command(os.Getenv("AB_REMOTE_BINARY"), "remote", "on", "--via", "tailscale-app", "--home", home)}
	for _, c := range commands {
		if e := c.Start(); e != nil {
			t.Fatal(e)
		}
	}
	for _, c := range commands {
		if e := c.Wait(); e != nil {
			t.Fatal(e)
		}
	}
	var selected map[string]bool
	if e := json.Unmarshal(invoke(t, "remote", "status", "--via", "both", "--home", home), &selected); e != nil {
		t.Fatal(e)
	}
	if !selected["relay"] || !selected["system"] {
		t.Fatal("concurrent on lost an adapter")
	}
	invoke(t, "remote", "off", "--via", "relay", "--home", home)
	if e := json.Unmarshal(invoke(t, "remote", "status", "--via", "both", "--home", home), &selected); e != nil {
		t.Fatal(e)
	}
	if selected["relay"] || !selected["system"] {
		t.Fatal("off disabled the other adapter")
	}
}
