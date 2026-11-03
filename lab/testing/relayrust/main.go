package main

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
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
)

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
func check(ok bool, name string) error {
	if !ok {
		return fmt.Errorf("FAIL %s", name)
	}
	fmt.Println("PASS", name)
	return nil
}
func address() (string, error) {
	l, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		return "", e
	}
	a := l.Addr().String()
	return a, l.Close()
}
func post(client *http.Client, endpoint string, input, output any, token string) int {
	data, _ := json.Marshal(input)
	req, _ := http.NewRequest("POST", endpoint, bytes.NewReader(data))
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	response, e := client.Do(req)
	if e != nil {
		fmt.Fprintln(os.Stderr, e)
		return 0
	}
	defer response.Body.Close()
	if output != nil {
		json.NewDecoder(response.Body).Decode(output)
	}
	return response.StatusCode
}
func run() error {
	if len(os.Args) != 3 {
		return fmt.Errorf("usage: relayrust /path/to/ab-relay /path/to/remote_client")
	}
	dir, e := os.MkdirTemp("", "ab-relay-rust-parity-")
	if e != nil {
		return e
	}
	defer os.RemoveAll(dir)
	public, key, e := ed25519.GenerateKey(rand.Reader)
	if e != nil {
		return e
	}
	template := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: "localhost"}, DNSNames: []string{"localhost"}, IPAddresses: []net.IP{net.ParseIP("127.0.0.1")}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature | x509.KeyUsageCertSign, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, IsCA: true, BasicConstraintsValid: true}
	der, e := x509.CreateCertificate(rand.Reader, template, template, public, key)
	if e != nil {
		return e
	}
	rootDer := append([]byte(nil), der...)
	controlPublic, controlKey, _ := ed25519.GenerateKey(rand.Reader)
	controlLeaf := *template
	controlLeaf.SerialNumber = big.NewInt(3)
	controlLeaf.Subject = pkix.Name{CommonName: "relay control"}
	controlLeaf.IsCA = false
	controlLeaf.KeyUsage = x509.KeyUsageDigitalSignature
	der, e = x509.CreateCertificate(rand.Reader, &controlLeaf, template, controlPublic, key)
	if e != nil {
		return e
	}
	private, e := x509.MarshalPKCS8PrivateKey(controlKey)
	if e != nil {
		return e
	}
	certPath, keyPath := filepath.Join(dir, "cert.pem"), filepath.Join(dir, "key.pem")
	os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0600)
	os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: private}), 0600)
	roots := x509.NewCertPool()
	cert, _ := x509.ParseCertificate(rootDer)
	roots.AddCert(cert)
	rootPath := filepath.Join(dir, "root.pem")
	os.WriteFile(rootPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: rootDer}), 0600)
	client := &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots}}, Timeout: 5 * time.Second}
	defer client.CloseIdleConnections()
	_, owner, _ := ed25519.GenerateKey(rand.Reader)
	_, stranger, _ := ed25519.GenerateKey(rand.Reader)
	config := filepath.Join(dir, "owners")
	os.WriteFile(config, []byte(base64.RawURLEncoding.EncodeToString(owner.Public().(ed25519.PublicKey))), 0600)
	path := filepath.Join(dir, "state.json")
	apiAddr, _ := address()
	publicAddr, _ := address()
	endpoint := "https://" + apiAddr
	var cmd *exec.Cmd
	stop := func() {
		if cmd != nil {
			cmd.Process.Signal(os.Interrupt)
			cmd.Wait()
			cmd = nil
		}
	}
	defer stop()
	start := func() error {
		cmd = exec.Command(os.Args[1], "--zone", "relay.test", "--state", path, "--addr", publicAddr, "--control-addr", apiAddr, "--control-cert", certPath, "--control-key", keyPath, "--owner-keys-file", config, "--idle-timeout", "2s")
		cmd.Env = append(os.Environ(), "APPLE_BRIDGE_RELAY_ADMIN=synthetic-admin")
		cmd.Stderr = os.Stderr
		if e := cmd.Start(); e != nil {
			return e
		}
		deadline := time.Now().Add(5 * time.Second)
		for time.Now().Before(deadline) {
			c, e := net.DialTimeout("tcp", apiAddr, 50*time.Millisecond)
			if e == nil {
				c.Close()
				return nil
			}
			time.Sleep(10 * time.Millisecond)
		}
		return fmt.Errorf("Rust relay readiness timeout")
	}
	if e := start(); e != nil {
		return e
	}
	enroll := func(key ed25519.PrivateKey, action string) (relay.Credential, int) {
		response, e := client.Get(endpoint + "/challenge")
		if e != nil {
			fmt.Fprintln(os.Stderr, e)
			return relay.Credential{}, 0
		}
		var c struct {
			Nonce string `json:"nonce"`
		}
		json.NewDecoder(response.Body).Decode(&c)
		response.Body.Close()
		in := relay.Enrollment{PublicKey: base64.RawURLEncoding.EncodeToString(key.Public().(ed25519.PublicKey)), Nonce: c.Nonce, Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, relay.EnrollmentMessage("relay.test", c.Nonce)))}
		var out relay.Credential
		status := post(client, endpoint+action, in, &out, "")
		return out, status
	}
	for _, action := range []string{"/enroll", "/rotate"} {
		_, status := enroll(stranger, action)
		if e := check(status == 403, "unknown denied "+action); e != nil {
			return e
		}
	}
	a, status := enroll(owner, "/enroll")
	if e := check(status == 200, "Go enrollment client against Rust"); e != nil {
		return e
	}
	limits := relay.Limits{Connections: 32, BytesPerSecond: 1048576, MonthlyBytes: 10737418240}
	store, e := relay.Open(path, "relay.test", limits)
	if e != nil {
		return e
	}
	d, ok := store.Device(a.Hostname)
	if e := check(ok && d.PublicKey == base64.RawURLEncoding.EncodeToString(owner.Public().(ed25519.PublicKey)), "Rust state opens in Go"); e != nil {
		return e
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	listener := relay.ListenTunnel(ctx, endpoint, client, owner, a)
	defer listener.Close()
	go func() {
		for {
			c, e := listener.Accept()
			if e != nil {
				return
			}
			go func() { defer c.Close(); io.Copy(c, c) }()
		}
	}()
	hello := func(host string) []byte {
		local, peer := net.Pipe()
		go func() { tls.Client(local, &tls.Config{ServerName: host}).Handshake(); local.Close() }()
		peer.SetReadDeadline(time.Now().Add(time.Second))
		header := make([]byte, 5)
		_, e := io.ReadFull(peer, header)
		if e != nil {
			peer.Close()
			return nil
		}
		body := make([]byte, int(header[3])<<8|int(header[4]))
		io.ReadFull(peer, body)
		peer.Close()
		return append(header, body...)
	}
	raw := hello(a.Hostname)
	var stream net.Conn
	deadline := time.Now().Add(5 * time.Second)
	for time.Now().Before(deadline) {
		stream, e = net.DialTimeout("tcp", publicAddr, time.Second)
		if e == nil {
			stream.SetDeadline(time.Now().Add(time.Second))
			_, e = stream.Write(raw)
			back := make([]byte, len(raw))
			_, e = io.ReadFull(stream, back)
			if e == nil && bytes.Equal(raw, back) {
				break
			}
			stream.Close()
		}
		time.Sleep(100 * time.Millisecond)
	}
	if e := check(e == nil, "Go TLS WebSocket smux v2 client and SNI passthrough"); e != nil {
		return e
	}
	payload := bytes.Repeat([]byte("ciphertext-marker"), 65536)
	stream.SetDeadline(time.Now().Add(15 * time.Second))
	done := make(chan error, 1)
	go func() { _, e := stream.Write(payload); done <- e }()
	back := make([]byte, len(payload))
	_, e = io.ReadFull(stream, back)
	writeErr := <-done
	stream.Close()
	if e := check(e == nil && writeErr == nil && bytes.Equal(payload, back), "smux v2 flow control beyond initial window"); e != nil {
		return e
	}
	listener.Close()
	remoteHome := filepath.Join(dir, "rust-client")
	os.MkdirAll(filepath.Join(remoteHome, "remote"), 0700)
	originPub, originKey, _ := ed25519.GenerateKey(rand.Reader)
	leaf := &x509.Certificate{SerialNumber: big.NewInt(2), DNSNames: []string{a.Hostname}, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(90 * 24 * time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	originDer, err := x509.CreateCertificate(rand.Reader, leaf, template, originPub, key)
	if err != nil {
		return err
	}
	originPrivate, _ := x509.MarshalPKCS8PrivateKey(originKey)
	os.WriteFile(filepath.Join(remoteHome, "remote", "relay-cert.pem"), pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: originDer}), 0600)
	os.WriteFile(filepath.Join(remoteHome, "remote", "relay-key.pem"), pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: originPrivate}), 0600)
	identity, _ := json.Marshal(map[string]any{"key": base64.StdEncoding.EncodeToString(owner), "credential": relay.Credential{}, "endpoint": endpoint})
	os.WriteFile(filepath.Join(remoteHome, "remote", "relay.json"), identity, 0600)
	origin := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { io.Copy(w, r.Body) }))
	defer origin.Close()
	rustClient := exec.Command(os.Args[2], remoteHome, endpoint, rootPath, origin.Listener.Addr().String())
	rustClient.Stderr = os.Stderr
	if err := rustClient.Start(); err != nil {
		return err
	}
	defer func() { rustClient.Process.Signal(os.Interrupt); rustClient.Wait() }()
	rustTransport := &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots}, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", publicAddr)
	}}
	defer rustTransport.CloseIdleConnections()
	rustRequest := &http.Client{Transport: rustTransport, Timeout: 10 * time.Second}
	deadline = time.Now().Add(10 * time.Second)
	var rustErr error
	for time.Now().Before(deadline) {
		response, err := rustRequest.Post("https://"+a.Hostname+"/", "application/octet-stream", bytes.NewReader(payload))
		rustErr = err
		if err == nil {
			body, err := io.ReadAll(response.Body)
			response.Body.Close()
			rustErr = err
			if err == nil && bytes.Equal(body, payload) {
				break
			}
			rustErr = fmt.Errorf("Rust client payload mismatch")
		}
		time.Sleep(100 * time.Millisecond)
	}
	if err := check(rustErr == nil, "Rust ab-remote enrollment TLS WebSocket smux client against Rust relay"); err != nil {
		return err
	}
	rustClient.Process.Signal(os.Interrupt)
	rustClient.Wait()
	b, status := enroll(owner, "/rotate")
	if e := check(status == 200 && a.Hostname == b.Hostname && a.Credential != b.Credential, "stable assignment credential rotation"); e != nil {
		return e
	}
	status = post(client, endpoint+"/policy", map[string]any{"hostname": a.Hostname, "limits": limits, "disabled": true}, nil, "synthetic-admin")
	if e := check(status == 204, "admin policy"); e != nil {
		return e
	}
	_, status = enroll(owner, "/rotate")
	stop()
	store, e = relay.Open(path, "relay.test", limits)
	if e != nil {
		return e
	}
	d, _ = store.Device(a.Hostname)
	if e := check(status == 200 && d.Disabled && d.Bytes > 0, "rotation preserves disabled and charged usage"); e != nil {
		return e
	}
	charged := d.Bytes
	if e := store.Policy(a.Hostname, limits, false, false); e != nil {
		return e
	}
	if e := start(); e != nil {
		return e
	}
	b, status = enroll(owner, "/rotate")
	if e := check(status == 200, "Go state reopens in Rust"); e != nil {
		return e
	}
	os.Remove(config)
	before, _ := os.ReadFile(path)
	_, status = enroll(owner, "/rotate")
	after, _ := os.ReadFile(path)
	if e := check(status == 403 && bytes.Equal(before, after), "missing owner file fails closed without state mutation"); e != nil {
		return e
	}
	os.WriteFile(config, []byte(base64.RawURLEncoding.EncodeToString(owner.Public().(ed25519.PublicKey))), 0600)
	status = post(client, endpoint+"/revoke", map[string]string{"hostname": a.Hostname}, nil, b.Credential)
	if e := check(status == 204, "device revoke"); e != nil {
		return e
	}
	stop()
	store, e = relay.Open(path, "relay.test", limits)
	if e != nil {
		return e
	}
	d, _ = store.Device(a.Hostname)
	if e := check(d.Revoked && d.Bytes == charged, "Go Rust state roundtrip preserves tombstone and usage"); e != nil {
		return e
	}
	info, e := os.Stat(path)
	if e != nil {
		return e
	}
	data, _ := os.ReadFile(path)
	if e := check(info.Mode().Perm() == 0600 && !strings.Contains(string(data), b.Credential), "private state excludes bearer credential"); e != nil {
		return e
	}
	if e := start(); e != nil {
		return e
	}
	_, status = enroll(owner, "/rotate")
	if e := check(status == 403, "revocation survives restart"); e != nil {
		return e
	}
	for i := 0; i < 31; i++ {
		response, e := client.Get(endpoint + "/challenge")
		if e != nil {
			return e
		}
		status = response.StatusCode
		response.Body.Close()
	}
	if e := check(status == 429, "challenge throttling"); e != nil {
		return e
	}
	for i := 0; i < 21; i++ {
		status = post(client, endpoint+"/rotate", relay.Enrollment{}, nil, "")
	}
	return check(status == 429, "shared enrollment rotation throttling")
}
