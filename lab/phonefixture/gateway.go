package phonefixture

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/json"
	"encoding/pem"
	"fmt"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/http/httputil"
	"os"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
)

type Gateway struct {
	Home   string
	URL    string
	Client *http.Client
	binary string
	env    []string
	ctx    context.Context
	cmd    *exec.Cmd
	done   chan error
	log    *os.File
	local  *http.Server
	public *http.Server
	mu     sync.Mutex
	config json.RawMessage
	etag   int
}

func Start(ctx context.Context, binary, home string) (*Gateway, error) {
	g := &Gateway{Home: home, URL: "https://node.fixture.test", binary: binary, ctx: ctx, config: json.RawMessage(`{"unrelated":"preserved"}`), etag: 1}
	if err := os.MkdirAll(home, 0700); err != nil {
		return nil, err
	}
	failed := true
	defer func() {
		if failed {
			g.Close()
		}
	}()
	socket := filepath.Join(filepath.Dir(home), "tailscale.sock")
	listener, err := net.Listen("unix", socket)
	if err != nil {
		return nil, err
	}
	g.local = &http.Server{ReadHeaderTimeout: 5 * time.Second, Handler: http.HandlerFunc(g.localAPI)}
	go g.local.Serve(listener)
	certificate, key, err := fixtureCertificate()
	if err != nil {
		return nil, err
	}
	certPath := filepath.Join(filepath.Dir(home), "fixture.pem")
	if err = os.WriteFile(certPath, certificate, 0600); err != nil {
		return nil, err
	}
	pair, err := tls.X509KeyPair(certificate, key)
	if err != nil {
		return nil, err
	}
	public, err := tls.Listen("tcp", "127.0.0.1:0", &tls.Config{MinVersion: tls.VersionTLS12, Certificates: []tls.Certificate{pair}})
	if err != nil {
		return nil, err
	}
	proxy := &httputil.ReverseProxy{Rewrite: func(request *httputil.ProxyRequest) {
		var lease struct {
			Target string `json:"target"`
		}
		raw, _ := os.ReadFile(filepath.Join(home, "remote", "lease.json"))
		_ = json.Unmarshal(raw, &lease)
		if target, err := http.NewRequest("GET", lease.Target, nil); err == nil {
			request.SetURL(target.URL)
		}
		request.Out.Host = "node.fixture.test"
		request.Out.Header.Set("X-Forwarded-Host", "node.fixture.test")
		request.Out.Header.Set("X-Forwarded-Proto", "https")
		request.Out.Header.Set("X-Forwarded-For", "198.51.100.4")
	}, FlushInterval: -1}
	g.public = &http.Server{ReadHeaderTimeout: 5 * time.Second, Handler: proxy}
	go g.public.Serve(public)
	roots := x509.NewCertPool()
	roots.AppendCertsFromPEM(certificate)
	transport := &http.Transport{TLSClientConfig: &tls.Config{MinVersion: tls.VersionTLS12, RootCAs: roots}, DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", public.Addr().String())
	}}
	g.Client = &http.Client{Transport: transport, Timeout: 15 * time.Second}
	g.env = append(os.Environ(), "APPLE_BRIDGE_SECRET_STORE=file", "APPLE_BRIDGE_TAILSCALE_SOCKET="+socket, "APPLE_BRIDGE_REMOTE_PROBE_ADDRESS="+public.Addr().String(), "APPLE_BRIDGE_RELAY_URL=https://127.0.0.1:1", "SSL_CERT_FILE="+certPath, "APPLE_BRIDGE_LAB_TLS_ROOT="+certPath, "APPLE_BRIDGE_LAB_PROBE_DURATION=1s", "APPLE_BRIDGE_LAB_REMOTE_DIAGNOSTICS="+filepath.Join(home, "remote-errors.log"))
	if err = g.Restart(); err != nil {
		return nil, err
	}
	if _, err = g.Command("remote", "on", "--via", "tailscale-app"); err != nil {
		detail, _ := os.ReadFile(filepath.Join(home, "remote-errors.log"))
		return nil, fmt.Errorf("%w; isolated remote diagnostic: %s", err, detail)
	}
	deadline := time.NewTimer(30 * time.Second)
	defer deadline.Stop()
	tick := time.NewTicker(25 * time.Millisecond)
	defer tick.Stop()
	for {
		var status struct {
			State string `json:"state"`
		}
		raw, _ := os.ReadFile(filepath.Join(home, "remote", "status.json"))
		if json.Unmarshal(raw, &status) == nil && status.State == "on" {
			break
		}
		select {
		case <-ctx.Done():
			return nil, ctx.Err()
		case <-deadline.C:
			return nil, fmt.Errorf("public ingress readiness timed out")
		case <-tick.C:
		}
	}
	failed = false
	return g, nil
}

func fixtureCertificate() ([]byte, []byte, error) {
	public, key, err := ed25519.GenerateKey(rand.Reader)
	if err != nil {
		return nil, nil, err
	}
	serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 120))
	if err != nil {
		return nil, nil, err
	}
	cert := &x509.Certificate{SerialNumber: serial, Subject: pkix.Name{CommonName: "node.fixture.test"}, DNSNames: []string{"node.fixture.test"}, IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().Add(time.Hour), IsCA: false, BasicConstraintsValid: true, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	der, err := x509.CreateCertificate(rand.Reader, cert, cert, public, key)
	if err != nil {
		return nil, nil, err
	}
	private, err := x509.MarshalPKCS8PrivateKey(key)
	return pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: private}), err
}

func (g *Gateway) localAPI(w http.ResponseWriter, request *http.Request) {
	g.mu.Lock()
	defer g.mu.Unlock()
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("ETag", strconv.Itoa(g.etag))
	if request.Method == "GET" && strings.HasPrefix(request.URL.Path, "/localapi/v0/status") {
		io.WriteString(w, `{"BackendState":"Running","Self":{"DNSName":"node.fixture.test.","CapMap":{"https":null,"funnel":null,"https://tailscale.com/cap/funnel-ports?ports=443,8443,10000":null}}}`)
		return
	}
	if request.URL.Path != "/localapi/v0/serve-config" {
		http.NotFound(w, request)
		return
	}
	if request.Method == "POST" {
		if request.Header.Get("If-Match") != strconv.Itoa(g.etag) {
			w.WriteHeader(412)
			return
		}
		raw, err := io.ReadAll(io.LimitReader(request.Body, 1<<20))
		if err != nil || !json.Valid(raw) {
			w.WriteHeader(400)
			return
		}
		g.config = raw
		g.etag++
		io.WriteString(w, `{}`)
		return
	}
	w.Write(g.config)
}

func (g *Gateway) Command(args ...string) (string, error) {
	ctx, cancel := context.WithTimeout(g.ctx, 100*time.Second)
	defer cancel()
	cmd := exec.CommandContext(ctx, g.binary, append(args, "--home", g.Home)...)
	cmd.Env = g.env
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("gateway %s: %w: %s", args[0], err, output)
	}
	return strings.TrimSpace(string(output)), nil
}

func (g *Gateway) Stop() {
	if g.cmd == nil {
		return
	}
	_ = g.cmd.Process.Signal(syscall.SIGTERM)
	select {
	case <-g.done:
	case <-time.After(10 * time.Second):
		_ = g.cmd.Process.Kill()
		<-g.done
	}
	g.cmd = nil
	g.log.Close()
}

func (g *Gateway) Restart() error {
	leasePath := filepath.Join(g.Home, "remote", "lease.json")
	_, leaseErr := os.Stat(leasePath)
	requireIngress := leaseErr == nil
	g.Stop()
	var err error
	g.log, err = os.OpenFile(filepath.Join(filepath.Dir(g.Home), "gateway.log"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		return err
	}
	reenabled := false
	started := time.Now()
	g.cmd = exec.CommandContext(g.ctx, g.binary, "start", "--role", "gateway", "--home", g.Home, "--api-addr", "127.0.0.1:0", "--peer-addr", "127.0.0.1:0")
	g.cmd.Env, g.cmd.Stdout, g.cmd.Stderr = g.env, g.log, g.log
	if err = g.cmd.Start(); err != nil {
		g.cmd = nil
		return err
	}
	g.done = make(chan error, 1)
	cmd := g.cmd
	go func() { g.done <- cmd.Wait() }()
	until := time.NewTimer(15 * time.Second)
	defer until.Stop()
	tick := time.NewTicker(25 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case err := <-g.done:
			g.cmd = nil
			return fmt.Errorf("gateway exited: %v", err)
		case <-g.ctx.Done():
			return g.ctx.Err()
		case <-until.C:
			return fmt.Errorf("gateway readiness timed out")
		case <-tick.C:
			configPath := filepath.Join(g.Home, "runtime.json")
			info, statErr := os.Stat(configPath)
			if statErr != nil || info.ModTime().Before(started) {
				continue
			}
			var runtime struct {
				APIAddress string `json:"api_address"`
			}
			raw, readErr := os.ReadFile(configPath)
			if readErr != nil || json.Unmarshal(raw, &runtime) != nil {
				continue
			}
			api, apiErr := net.DialTimeout("tcp", runtime.APIAddress, 100*time.Millisecond)
			if apiErr != nil {
				continue
			}
			api.Close()
			socket, dialErr := net.DialTimeout("unix", filepath.Join(g.Home, "run", "peer.sock"), 100*time.Millisecond)
			if dialErr != nil {
				continue
			}
			socket.Close()
			if requireIngress {
				if !reenabled {
					if _, enableErr := g.Command("remote", "on", "--via", "tailscale-app"); enableErr != nil {
						return enableErr
					}
					reenabled = true
				}
				if leaseInfo, leaseErr := os.Stat(leasePath); leaseErr != nil || leaseInfo.ModTime().Before(started) {
					continue
				}
				response, probeErr := g.Client.Get(g.URL + "/")
				if probeErr != nil {
					continue
				}
				response.Body.Close()
				if response.StatusCode == 502 || response.StatusCode == 503 {
					continue
				}
			}
			return nil
		}
	}
}

func (g *Gateway) Close() {
	g.Stop()
	if g.Client != nil {
		g.Client.CloseIdleConnections()
	}
	if g.public != nil {
		g.public.Close()
	}
	if g.local != nil {
		g.local.Close()
	}
}
