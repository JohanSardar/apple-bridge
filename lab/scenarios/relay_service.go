package scenarios

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base64"
	"encoding/json"
	"errors"
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
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
)

type relayWire struct {
	mu     sync.Mutex
	marker []byte
	leaked bool
	bytes  int
	opens  int
	active int
}

func (w *relayWire) observe(b []byte) {
	w.mu.Lock()
	defer w.mu.Unlock()
	w.leaked = w.leaked || bytes.Contains(b, w.marker)
	w.bytes += len(b)
}

type relayPipe struct {
	net.Conn
	wire *relayWire
	once *sync.Once
}

func (p relayPipe) Close() error {
	err := p.Conn.Close()
	p.once.Do(func() { p.wire.mu.Lock(); p.wire.active--; p.wire.mu.Unlock() })
	return err
}
func (p relayPipe) Read(b []byte) (int, error) {
	n, e := p.Conn.Read(b)
	p.wire.observe(b[:n])
	return n, e
}
func (p relayPipe) Write(b []byte) (int, error) { p.wire.observe(b); return p.Conn.Write(b) }

type relayTestTunnel struct {
	address string
	wire    *relayWire
}

func (t relayTestTunnel) Open(ctx context.Context) (io.ReadWriteCloser, error) {
	c, e := (&net.Dialer{}).DialContext(ctx, "tcp", t.address)
	if e != nil {
		return nil, e
	}
	t.wire.mu.Lock()
	t.wire.opens++
	t.wire.active++
	t.wire.mu.Unlock()
	return relayPipe{c, t.wire, &sync.Once{}}, nil
}

type relayBlockedTunnel struct{ wire *relayWire }

func (t relayBlockedTunnel) Open(ctx context.Context) (io.ReadWriteCloser, error) {
	local, peer := net.Pipe()
	context.AfterFunc(ctx, func() { peer.Close() })
	t.wire.mu.Lock()
	t.wire.opens++
	t.wire.active++
	t.wire.mu.Unlock()
	return relayPipe{local, t.wire, &sync.Once{}}, nil
}

type relayLab struct {
	once     sync.Once
	server   *relay.Server
	api      *httptest.Server
	address  string
	stop     context.CancelFunc
	done     chan error
	listener net.Listener
}

func startRelayLab(ctx context.Context, store *relay.Store) (*relayLab, error) {
	l, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	bounded, cancel := context.WithCancel(ctx)
	r := &relayLab{listener: l, server: relay.New(store), address: l.Addr().String(), stop: cancel, done: make(chan error, 1)}
	r.server.IdleTimeout = 2 * time.Second
	r.api = httptest.NewServer(r.server.Handler("synthetic-admin"))
	go func() { r.done <- r.server.Serve(bounded, l) }()
	return r, nil
}
func (r *relayLab) close() {
	if r != nil {
		r.once.Do(func() { r.stop(); r.api.Close(); <-r.done })
	}
}
func relayPost(client *http.Client, url string, in, out any, token string) int {
	data, _ := json.Marshal(in)
	req, _ := http.NewRequest("POST", url, bytes.NewReader(data))
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	resp, err := client.Do(req)
	if err != nil {
		return 0
	}
	defer resp.Body.Close()
	if out != nil {
		json.NewDecoder(resp.Body).Decode(out)
	}
	return resp.StatusCode
}
func relayEnroll(api string, key ed25519.PrivateKey, host string) (relay.Credential, int) {
	resp, err := http.Get(api + "/challenge")
	if err != nil {
		return relay.Credential{}, 0
	}
	var challenge struct {
		Nonce string `json:"nonce"`
	}
	json.NewDecoder(resp.Body).Decode(&challenge)
	resp.Body.Close()
	request := relay.Enrollment{PublicKey: base64.RawURLEncoding.EncodeToString(key.Public().(ed25519.PublicKey)), Nonce: challenge.Nonce, Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, relay.EnrollmentMessage("relay.test", challenge.Nonce))), Hostname: host}
	var c relay.Credential
	code := relayPost(http.DefaultClient, api+"/enroll", request, &c, "")
	return c, code
}
func relayHello(host string) []byte {
	client, peer := net.Pipe()
	done := make(chan struct{})
	go func() { defer close(done); tls.Client(client, &tls.Config{ServerName: host}).Handshake() }()
	peer.SetReadDeadline(time.Now().Add(time.Second))
	header := make([]byte, 5)
	_, err := io.ReadFull(peer, header)
	var body []byte
	if err == nil {
		body = make([]byte, int(header[3])<<8|int(header[4]))
		_, err = io.ReadFull(peer, body)
	}
	peer.Close()
	client.Close()
	<-done
	if err != nil {
		return nil
	}
	return append(header, body...)
}
func relayStreamClosed(err error) bool {
	var timeout net.Error
	if errors.As(err, &timeout) && timeout.Timeout() {
		return false
	}
	return errors.Is(err, io.EOF) || errors.Is(err, io.ErrUnexpectedEOF) || errors.Is(err, net.ErrClosed) || errors.Is(err, syscall.ECONNRESET)
}
func relayDrained(w *relayWire) bool {
	deadline := time.Now().Add(time.Second)
	for time.Now().Before(deadline) {
		w.mu.Lock()
		active := w.active
		w.mu.Unlock()
		if active == 0 {
			return true
		}
		time.Sleep(time.Millisecond)
	}
	return false
}
func runRelayService(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), GitSHA: strings.TrimSpace(string(sha)), RunID: "relay-" + strings.ToLower(strings.TrimPrefix(s.ID, "RELAY-")), Hosts: []string{"air"}, NetworkPath: "loopback TLS origins through raw relay tunnel interface"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if level == "real" {
		e.Status = "limited"
		e.Assertions = append(e.Assertions, Assertion{Name: "public_relay_readiness", Status: "limited", Detail: "NS delegation absent; trusted control TLS and VM DNS scopes/IAM require operator setup"})
		return e
	}
	dir, err := os.MkdirTemp("", "ab-relay-lab-")
	if err != nil {
		add("fixture", false)
		return e
	}
	defer os.RemoveAll(dir)
	path := filepath.Join(dir, "relay.json")
	limits := relay.Limits{Connections: 2, BytesPerSecond: 1048576, MonthlyBytes: 10485760}
	store, err := relay.Open(path, "relay.test", limits)
	if err != nil {
		add("store", false)
		return e
	}
	lab, err := startRelayLab(ctx, store)
	if err != nil {
		add("listener", false)
		return e
	}
	defer func() { lab.close() }()
	_, key1, _ := ed25519.GenerateKey(rand.Reader)
	_, key2, _ := ed25519.GenerateKey(rand.Reader)
	a, code := relayEnroll(lab.api.URL, key1, "")
	b, code2 := relayEnroll(lab.api.URL, key2, "")
	add("two_distinct_device_enrollments", code == 200 && code2 == 200 && a.Hostname != b.Hostname)
	if code != 200 || code2 != 200 {
		return e
	}
	switch s.ID {
	case "RELAY-ACME":
		relayDNSProof(ctx, lab, key1, a, b, add)
		if err := relayPebble(ctx, lab, key1, a, false, add); err != nil {
			add("pebble_issue_renew: "+err.Error(), false)
		}
	case "RELAY-OWN-DOMAIN":
		relayOwnDomain(ctx, lab, key1, a, b, add)
		if err := relayPebble(ctx, lab, key1, a, true, add); err != nil {
			add("own_domain_certificate: "+err.Error(), false)
		}
	case "RELAY-STREAMS":
		relayStreams(ctx, lab, key1, a, add)
	case "RELAY-ENROLL":
		lab.close()
		store, err = relay.Open(path, "relay.test", limits)
		if err != nil {
			add("restart", false)
			return e
		}
		lab, err = startRelayLab(ctx, store)
		if err != nil {
			add("restart", false)
			return e
		}
		rotated, status := relayEnroll(lab.api.URL, key1, "")
		add("stable_hostname_after_restart", status == 200 && rotated.Hostname == a.Hostname && rotated.Credential != a.Credential)
		tunnel := relayTestTunnel{address: "127.0.0.1:1", wire: &relayWire{}}
		_, err = lab.server.Attach(a.Hostname, a.Credential, tunnel)
		add("old_credential_refused", err != nil)
		detach, err := lab.server.Attach(a.Hostname, rotated.Credential, tunnel)
		add("rotated_credential_accepted", err == nil)
		if detach != nil {
			defer detach()
		}
		_, status = relayEnroll(lab.api.URL, key2, a.Hostname)
		add("hostname_claim_by_other_key_refused", status == 403)
		status = relayPost(http.DefaultClient, lab.api.URL+"/enroll", relay.Enrollment{PublicKey: base64.RawURLEncoding.EncodeToString(key1.Public().(ed25519.PublicKey))}, nil, "")
		add("key_possession_required", status == 403)
		status = relayPost(http.DefaultClient, lab.api.URL+"/revoke", map[string]string{"hostname": a.Hostname}, nil, rotated.Credential)
		add("revoke", status == 204)
		_, err = lab.server.Attach(a.Hostname, rotated.Credential, tunnel)
		_, status = relayEnroll(lab.api.URL, key1, "")
		add("revocation_blocks_tunnel_and_reenrollment", err != nil && status == 403)
		restarted, err := relay.Open(path, "relay.test", limits)
		add("revocation_persists", err == nil)
		if err == nil {
			d, _ := restarted.Device(a.Hostname)
			add("revoked_state", d.Revoked)
		}
		info, err := os.Stat(path)
		data, _ := os.ReadFile(path)
		add("private_state_no_bearer_credentials", err == nil && info.Mode().Perm() == 0600 && !bytes.Contains(data, []byte(rotated.Credential)))
	case "RELAY-PASSTHROUGH", "RELAY-LIMITS":
		marker := []byte("ab-synthetic-secret-marker-" + s.ID)
		wire := &relayWire{marker: marker}
		roots := x509.NewCertPool()
		origin := func(host, label string) *httptest.Server {
			public, private, _ := ed25519.GenerateKey(rand.Reader)
			template := &x509.Certificate{SerialNumber: big.NewInt(time.Now().UnixNano()), Subject: pkix.Name{CommonName: host}, DNSNames: []string{host}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, IsCA: true, BasicConstraintsValid: true}
			der, _ := x509.CreateCertificate(rand.Reader, template, template, public, private)
			cert, _ := x509.ParseCertificate(der)
			roots.AddCert(cert)
			server := httptest.NewUnstartedServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				payload, _ := io.ReadAll(r.Body)
				if !bytes.Equal(payload, marker) {
					w.WriteHeader(400)
					return
				}
				w.Write([]byte(label))
				w.Write(bytes.Repeat([]byte("x"), 8192))
			}))
			server.TLS = &tls.Config{Certificates: []tls.Certificate{{Certificate: [][]byte{der}, PrivateKey: private}}}
			server.StartTLS()
			return server
		}
		first := origin(a.Hostname, "first")
		defer first.Close()
		second := origin(b.Hostname, "second")
		defer second.Close()
		attach := func(c relay.Credential, address string) bool {
			_, err := lab.server.Attach(c.Hostname, c.Credential, relayTestTunnel{address: address, wire: wire})
			return err == nil
		}
		add("authenticated_device_tunnels", attach(a, first.Listener.Addr().String()) && attach(b, second.Listener.Addr().String()))
		_, wrongCredential := lab.server.Attach(a.Hostname, b.Credential, relayTestTunnel{address: second.Listener.Addr().String(), wire: wire})
		add("cross_device_credential_refused", wrongCredential != nil)
		request := func(c relay.Credential) (string, error) {
			tr := &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots}, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				return (&net.Dialer{}).DialContext(ctx, "tcp", lab.address)
			}}
			defer tr.CloseIdleConnections()
			client := &http.Client{Transport: tr, Timeout: 5 * time.Second}
			resp, err := client.Post("https://"+c.Hostname+"/", "application/octet-stream", bytes.NewReader(marker))
			if err != nil {
				return "", err
			}
			defer resp.Body.Close()
			body, err := io.ReadAll(resp.Body)
			return string(body), err
		}
		one, err1 := request(a)
		two, err2 := request(b)
		add("hosts_reach_only_their_bridge", err1 == nil && err2 == nil && strings.HasPrefix(one, "first") && strings.HasPrefix(two, "second"))
		wire.mu.Lock()
		before := wire.opens
		wire.mu.Unlock()
		unknown, unknownErr := net.DialTimeout("tcp", lab.address, time.Second)
		if unknownErr == nil {
			unknown.SetDeadline(time.Now().Add(time.Second))
			_, unknownErr = unknown.Write(relayHello("unknown.relay.test"))
			if unknownErr == nil {
				n, readErr := unknown.Read(make([]byte, 1))
				add("unknown_sni_closed", n == 0 && relayStreamClosed(readErr))
			} else {
				add("unknown_sni_closed", false)
			}
			unknown.Close()
		} else {
			add("unknown_sni_closed", false)
		}
		wire.mu.Lock()
		add("unknown_sni_opens_no_tunnel", wire.opens == before)
		wire.mu.Unlock()
		if s.ID == "RELAY-LIMITS" {
			policy := func(l relay.Limits, disabled bool) bool {
				return relayPost(http.DefaultClient, lab.api.URL+"/policy", map[string]any{"hostname": a.Hostname, "limits": l, "disabled": disabled}, nil, "synthetic-admin") == 204
			}
			small := limits
			small.Connections = 1
			add("connection_policy", policy(small, false) && attach(a, first.Listener.Addr().String()))
			stalled, stallErr := net.DialTimeout("tcp", lab.address, time.Second)
			if stallErr == nil {
				stalled.SetDeadline(time.Now().Add(3 * time.Second))
				_, stallErr = stalled.Write(relayHello(a.Hostname))
				if stallErr == nil {
					n, drainErr := io.Copy(io.Discard, stalled)
					add("stalled_tls_releases_slot", drainErr == nil && n > 0 && relayDrained(wire))
				} else {
					add("stalled_tls_releases_slot", false)
				}
				stalled.Close()
			} else {
				add("stalled_tls_releases_slot", false)
			}
			_, err = request(a)
			add("stalled_tls_slot_reusable", err == nil)
			add("stalled_tls_probe_drains", relayDrained(wire))
			lab.server.Disconnect(a.Hostname)
			_, blockedErr := lab.server.Attach(a.Hostname, a.Credential, relayBlockedTunnel{wire})
			wire.mu.Lock()
			opened := wire.opens
			wire.mu.Unlock()
			blocked, dialErr := net.DialTimeout("tcp", lab.address, time.Second)
			if blockedErr == nil && dialErr == nil {
				blocked.SetDeadline(time.Now().Add(3 * time.Second))
				_, writeErr := blocked.Write(relayHello(a.Hostname))
				n, drainErr := io.Copy(io.Discard, blocked)
				drained := relayDrained(wire)
				wire.mu.Lock()
				admitted := wire.opens == opened+1
				wire.mu.Unlock()
				add("blocked_tunnel_write_releases_slot", writeErr == nil && drainErr == nil && n == 0 && admitted && drained)
			} else {
				add("blocked_tunnel_write_releases_slot", false)
			}
			if blocked != nil {
				blocked.Close()
			}
			lab.server.Disconnect(a.Hostname)
			add("restore_origin_after_idle", attach(a, first.Listener.Addr().String()))
			_, err = request(a)
			add("slot_reusable_after_idle", err == nil)
			add("idle_probe_drains", relayDrained(wire))
			held, err := tls.DialWithDialer(&net.Dialer{Timeout: 2 * time.Second}, "tcp", lab.address, &tls.Config{ServerName: a.Hostname, RootCAs: roots})
			add("first_connection_accepted", err == nil)
			_, err = request(a)
			add("excess_connection_rejected", err != nil)
			if held != nil {
				defer held.Close()
			}
			add("kill_switch", policy(small, true))
			if held != nil {
				held.SetReadDeadline(time.Now().Add(time.Second))
				n, readErr := held.Read(make([]byte, 1))
				add("kill_closes_active_connection", n == 0 && relayStreamClosed(readErr))
				add("kill_drains_stream", relayDrained(wire))
				held.Close()
			}
			_, err = lab.server.Attach(a.Hostname, a.Credential, relayTestTunnel{address: first.Listener.Addr().String(), wire: wire})
			add("kill_blocks_reattach", err != nil)
			slow := limits
			slow.BytesPerSecond = 32768
			add("bandwidth_policy", policy(slow, false) && attach(a, first.Listener.Addr().String()))
			start := time.Now()
			_, err = request(a)
			add("aggregate_bandwidth_throttled", err == nil && time.Since(start) >= 250*time.Millisecond)
			exhausted := limits
			lab.server.Disconnect(a.Hostname)
			used, _ := store.Device(a.Hostname)
			exhausted.MonthlyBytes = used.Bytes + 1
			add("monthly_policy", policy(exhausted, false) && attach(a, first.Listener.Addr().String()))
			_, err = request(a)
			add("monthly_budget_rejects", err != nil)
			reopened, err := relay.Open(path, "relay.test", limits)
			if err == nil {
				d, _ := reopened.Device(a.Hostname)
				add("monthly_usage_persisted", d.Bytes == exhausted.MonthlyBytes)
			} else {
				add("monthly_usage_persisted", false)
			}
			other, err := request(b)
			add("other_device_unaffected", err == nil && strings.HasPrefix(other, "second"))
		}
		if s.ID == "RELAY-PASSTHROUGH" {
			held, dialErr := tls.DialWithDialer(&net.Dialer{Timeout: time.Second}, "tcp", lab.address, &tls.Config{ServerName: b.Hostname, RootCAs: roots})
			add("listener_exit_active_stream", dialErr == nil)
			lab.listener.Close()
			select {
			case result := <-lab.done:
				add("listener_exit_returns_promptly", result != nil)
				lab.done <- result
			case <-time.After(time.Second):
				add("listener_exit_returns_promptly", false)
			}
			if held != nil {
				held.SetReadDeadline(time.Now().Add(time.Second))
				n, readErr := held.Read(make([]byte, 1))
				add("listener_exit_closes_stream", n == 0 && relayStreamClosed(readErr))
				held.Close()
			}
			add("listener_exit_drains_streams", relayDrained(wire))
		}
		wire.mu.Lock()
		add("relay_tunnel_buffers_contain_ciphertext_only", wire.bytes > 0 && !wire.leaked)
		wire.mu.Unlock()
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
