package scenarios

import (
	"bufio"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"encoding/pem"
	"io"
	"log"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

const compatHost = "gateway.compat.test"

func compatDial(ctx context.Context, address string, roots *x509.CertPool) (*websocket.Conn, []byte, error) {
	client := &http.Client{Transport: &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS13}, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", address)
	}}, Timeout: 5 * time.Second}
	conn, response, err := websocket.Dial(ctx, "wss://"+compatHost+"/v1/feed", &websocket.DialOptions{HTTPClient: client})
	if err != nil {
		return nil, nil, err
	}
	binding, err := trust.RelayBinding(response.TLS)
	if err != nil {
		conn.CloseNow()
		return nil, nil, err
	}
	return conn, binding, nil
}
func relayLegacyCompat(ctx context.Context, dir string, add func(string, bool)) {
	old, present := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
	os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	defer func() {
		if present {
			os.Setenv("APPLE_BRIDGE_SECRET_STORE", old)
		} else {
			os.Unsetenv("APPLE_BRIDGE_SECRET_STORE")
		}
	}()
	ctx, cancel := context.WithTimeout(ctx, 40*time.Second)
	defer cancel()
	gateway, err := trust.Open(filepath.Join(dir, "gateway"))
	if err != nil {
		add("gateway", false)
		return
	}
	pairing := httptest.NewUnstartedServer(gateway.PairHandler())
	pairing.TLS = &tls.Config{Certificates: []tls.Certificate{gateway.Certificate}, ClientAuth: tls.RequireAnyClientCert, MinVersion: tls.VersionTLS13}
	pairing.StartTLS()
	defer pairing.Close()
	pair := func(name string) *trust.Authority {
		replica, err := trust.Open(filepath.Join(dir, name))
		if err != nil {
			return nil
		}
		code, err := gateway.Pair(strings.TrimPrefix(pairing.URL, "https://"), []contracts.Grant{"reminders:read"})
		if err != nil || replica.Join(ctx, code) != nil {
			return nil
		}
		return replica
	}
	legacy, upgraded := pair("legacy-replica"), pair("upgraded-replica")
	stranger, strangerErr := trust.Open(filepath.Join(dir, "stranger"))
	add("legacy_and_upgraded_replicas_paired", legacy != nil && upgraded != nil && strangerErr == nil)
	if legacy == nil || upgraded == nil || strangerErr != nil {
		return
	}
	public, private, _ := ed25519.GenerateKey(rand.Reader)
	ca := &x509.Certificate{SerialNumber: big.NewInt(1), IsCA: true, BasicConstraintsValid: true, KeyUsage: x509.KeyUsageCertSign, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour)}
	der, _ := x509.CreateCertificate(rand.Reader, ca, ca, public, private)
	ca, _ = x509.ParseCertificate(der)
	roots := x509.NewCertPool()
	roots.AddCert(ca)
	caPath := filepath.Join(dir, "ca.pem")
	os.WriteFile(caPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0600)
	issue := func(serial int64) tls.Certificate {
		pub, key, _ := ed25519.GenerateKey(rand.Reader)
		cert := &x509.Certificate{SerialNumber: big.NewInt(serial), DNSNames: []string{compatHost}, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, NotBefore: ca.NotBefore, NotAfter: ca.NotAfter}
		raw, _ := x509.CreateCertificate(rand.Reader, cert, ca, pub, private)
		return tls.Certificate{Certificate: [][]byte{raw, der}, PrivateKey: key}
	}
	start := func(handler http.Handler, cert tls.Certificate) *httptest.Server {
		server := httptest.NewUnstartedServer(handler)
		server.Config.ErrorLog = log.New(io.Discard, "", 0)
		server.TLS = &tls.Config{Certificates: []tls.Certificate{cert}, MinVersion: tls.VersionTLS13}
		server.StartTLS()
		return server
	}
	feed := &replication.Server{Authority: gateway}
	origin := start(feed.RelayHandler(), issue(2))
	defer origin.Close()
	type target struct {
		name    string
		address string
	}
	targets := []target{{"go", origin.Listener.Addr().String()}}
	if fixture := os.Getenv("AB_RELAY_FIXTURE"); fixture != "" {
		originCert := issue(3)
		key, keyErr := x509.MarshalPKCS8PrivateKey(originCert.PrivateKey)
		certPath, keyPath := filepath.Join(dir, "tls.pem"), filepath.Join(dir, "tls-key.pem")
		if keyErr == nil {
			os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: originCert.Certificate[0]}), 0600)
			os.WriteFile(keyPath, pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: key}), 0600)
		}
		cmd := exec.CommandContext(ctx, filepath.Join(filepath.Dir(fixture), "relay2_fixture"), filepath.Join(dir, "gateway"), certPath, keyPath)
		cmd.Stderr = os.Stderr
		stdout, pipeErr := cmd.StdoutPipe()
		if pipeErr == nil {
			pipeErr = cmd.Start()
		}
		add("rust_gateway_started", keyErr == nil && pipeErr == nil)
		if keyErr != nil || pipeErr != nil {
			return
		}
		defer func() { cmd.Process.Kill(); cmd.Wait() }()
		address, readErr := bufio.NewReader(stdout).ReadString('\n')
		add("rust_gateway_ready", readErr == nil)
		if readErr != nil {
			return
		}
		targets = append(targets, target{"rust", strings.TrimSpace(address)})
	}
	for i, t := range targets {
		if i > 0 {
			legacy, upgraded = pair("legacy-replica-"+t.name), pair("upgraded-replica-"+t.name)
			if legacy == nil || upgraded == nil {
				add(t.name+"_replicas_paired", false)
				continue
			}
		}
		relayCompatTarget(ctx, t.name, t.address, roots, gateway, legacy, upgraded, stranger, add)
	}
	relayNoFallback(ctx, dir, gateway, upgraded, start, issue, roots, add)
}
func relayCompatTarget(ctx context.Context, name, address string, roots *x509.CertPool, gateway, legacy, upgraded, stranger *trust.Authority, add func(string, bool)) {
	dial := func() *websocket.Conn {
		conn, _, err := compatDial(ctx, address, roots)
		if err != nil {
			return nil
		}
		return conn
	}
	attempt1 := func(replica *trust.Authority) (trust.Principal, error) {
		conn := dial()
		if conn == nil {
			return trust.Principal{}, contracts.ErrInvalid
		}
		defer conn.CloseNow()
		return replica.RelayClient(ctx, conn, gateway.Fingerprint)
	}
	attempt2 := func(replica *trust.Authority) (trust.Principal, error) {
		conn, binding, err := compatDial(ctx, address, roots)
		if err != nil {
			return trust.Principal{}, err
		}
		defer conn.CloseNow()
		return replica.Relay2Client(ctx, conn, gateway.Fingerprint, nil, binding)
	}
	principal, err := attempt1(legacy)
	add(name+"_relay1_legacy_replica_authenticates_unchanged", err == nil && relayPrincipalID(principal) && !principal.Relay2)
	_, err = attempt1(stranger)
	add(name+"_relay1_unpaired_replica_rejected", err != nil)
	principal, err = attempt2(upgraded)
	add(name+"_relay2_replica_authenticates_and_is_marked", err == nil && relayPrincipalID(principal) && principal.Relay2)
	_, err = attempt1(upgraded)
	add(name+"_relay2_marked_replica_never_accepted_on_relay1", err != nil)
	principal, err = attempt1(legacy)
	add(name+"_relay1_legacy_replica_unaffected_by_other_peer_upgrade", err == nil && relayPrincipalID(principal) && !principal.Relay2)
	_, err = attempt2(stranger)
	add(name+"_relay2_unpaired_replica_rejected", err != nil)
	principal, err = attempt2(legacy)
	add(name+"_relay1_replica_upgrades_to_relay2_in_place", err == nil && principal.Relay2)
	_, err = attempt1(legacy)
	add(name+"_upgraded_replica_relay1_rejected_afterwards", err != nil)
}
func relayNoFallback(ctx context.Context, dir string, gateway, replica *trust.Authority, start func(http.Handler, tls.Certificate) *httptest.Server, issue func(int64) tls.Certificate, roots *x509.CertPool, add func(string, bool)) {
	var mutex sync.Mutex
	connections, frames, legacyHello := 0, 0, false
	only1 := start(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		mutex.Lock()
		connections++
		mutex.Unlock()
		bounded, cancel := context.WithTimeout(r.Context(), 2*time.Second)
		defer cancel()
		for {
			var raw json.RawMessage
			if wsjson.Read(bounded, conn, &raw) != nil {
				return
			}
			var probe map[string]json.RawMessage
			json.Unmarshal(raw, &probe)
			mutex.Lock()
			frames++
			if _, versioned := probe["version"]; !versioned {
				legacyHello = true
			}
			mutex.Unlock()
			conn.Close(websocket.StatusPolicyViolation, "relay/1 only")
			return
		}
	}), issue(4))
	defer only1.Close()
	conn, binding, err := compatDial(ctx, only1.Listener.Addr().String(), roots)
	if err == nil {
		_, err = replica.Relay2Client(ctx, conn, gateway.Fingerprint, nil, binding)
		conn.CloseNow()
	}
	time.Sleep(300 * time.Millisecond)
	mutex.Lock()
	defer mutex.Unlock()
	add("relay2_required_client_fails_against_relay1_only_gateway", err != nil)
	add("relay2_required_client_never_falls_back_to_relay1", connections == 1 && frames == 1 && !legacyHello)
}
