package scenarios

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"io"
	"log"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	relay "github.com/JohanSardar/apple-bridge/lab/relayfixture"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func relayTLSSplice(ctx context.Context, dir string, add func(string, bool)) {
	old, present := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
	os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	defer func() {
		if present {
			os.Setenv("APPLE_BRIDGE_SECRET_STORE", old)
		} else {
			os.Unsetenv("APPLE_BRIDGE_SECRET_STORE")
		}
	}()
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	gateway, err := trust.Open(filepath.Join(dir, "gateway"))
	if err != nil {
		add("gateway", false)
		return
	}
	replica, err := trust.Open(filepath.Join(dir, "replica"))
	if err != nil {
		add("replica", false)
		return
	}
	pairing := httptest.NewUnstartedServer(gateway.PairHandler())
	pairing.TLS = &tls.Config{Certificates: []tls.Certificate{gateway.Certificate}, ClientAuth: tls.RequireAnyClientCert, MinVersion: tls.VersionTLS13}
	pairing.StartTLS()
	defer pairing.Close()
	code, err := gateway.Pair(strings.TrimPrefix(pairing.URL, "https://"), []contracts.Grant{"reminders:read"})
	if err == nil {
		err = replica.Join(ctx, code)
	}
	add("genuine_paired_identities", err == nil)
	if err != nil {
		return
	}
	store, err := state.Open(filepath.Join(dir, "feed"))
	if err != nil {
		add("feed_store", false)
		return
	}
	defer store.Close()
	marker := "synthetic-splice-feed-marker"
	head := store.Head()
	_, err = store.ApplyBatch(ctx, contracts.Batch{Epoch: head.Epoch, SubscriptionRevision: head.SubscriptionRevision, After: head.Sequence, Through: head.Sequence + 1, Changes: []contracts.Change{{Sequence: head.Sequence + 1, Op: "upsert", Record: contracts.Record{Collection: "reminders.reminder", ID: "synthetic", Version: 1, Source: "reminders", Data: json.RawMessage(`{"title":"` + marker + `"}`), ObservedAt: time.Now().UTC()}}}})
	add("seeded_feed", err == nil)
	if err != nil {
		return
	}
	relayStore, err := relay.Open(filepath.Join(dir, "relay-state.json"), "relay.test", relay.Limits{Connections: 32, BytesPerSecond: 1048576, MonthlyBytes: 10737418240})
	if err != nil {
		add("relay_fixture", false)
		return
	}
	lab, err := startRelayLab(ctx, relayStore)
	if err != nil {
		add("relay_listener", false)
		return
	}
	defer lab.close()
	_, deviceKey, _ := ed25519.GenerateKey(rand.Reader)
	credential, status := relayEnroll(lab.api.URL, deviceKey, "")
	add("splice_origin_enrolled", status == 200)
	if status != 200 {
		return
	}
	victimHost := credential.Hostname
	public, private, _ := ed25519.GenerateKey(rand.Reader)
	ca := &x509.Certificate{SerialNumber: big.NewInt(1), IsCA: true, BasicConstraintsValid: true, KeyUsage: x509.KeyUsageCertSign, NotBefore: time.Now().Add(-time.Hour), NotAfter: time.Now().Add(time.Hour)}
	der, _ := x509.CreateCertificate(rand.Reader, ca, ca, public, private)
	ca, _ = x509.ParseCertificate(der)
	roots := x509.NewCertPool()
	roots.AddCert(ca)
	issue := func(serial int64) tls.Certificate {
		pub, key, _ := ed25519.GenerateKey(rand.Reader)
		cert := &x509.Certificate{SerialNumber: big.NewInt(serial), DNSNames: []string{victimHost}, KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}, NotBefore: ca.NotBefore, NotAfter: ca.NotAfter}
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
	feed := &replication.Server{Authority: gateway, Store: store}
	origin := start(feed.RelayHandler(), issue(2))
	defer origin.Close()
	client := func(address string, config *tls.Config) *http.Client {
		return &http.Client{Transport: &http.Transport{TLSClientConfig: config, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
			return (&net.Dialer{}).DialContext(ctx, "tcp", address)
		}}, Timeout: 5 * time.Second}
	}
	var observed atomic.Bool
	var frames atomic.Int32
	detach, err := lab.server.Attach(credential.Hostname, credential.Credential, relayTestTunnel{address: origin.Listener.Addr().String(), wire: &relayWire{}})
	add("splice_origin_tunnel_attached", err == nil)
	if err != nil {
		return
	}
	defer detach()
	upstreamClient := client(lab.address, &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS13})
	defer upstreamClient.CloseIdleConnections()
	splicer := start(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		upstream, _, err := websocket.Dial(r.Context(), "wss://"+victimHost+"/v1/feed", &websocket.DialOptions{HTTPClient: upstreamClient})
		if err != nil {
			http.Error(w, "upstream", 502)
			return
		}
		defer upstream.CloseNow()
		downstream, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer downstream.CloseNow()
		forward := func(dst, src *websocket.Conn) {
			for {
				kind, data, err := src.Read(r.Context())
				if err != nil {
					return
				}
				for _, name := range []string{"AuthHello", "AuthChallenge", "AuthProof", "Authenticated"} {
					if bytes.Contains(data, []byte(`"t":"`+name+`"`)) {
						frames.Add(1)
					}
				}
				if bytes.Contains(data, []byte(marker)) {
					observed.Store(true)
				}
				if dst.Write(r.Context(), kind, data) != nil {
					return
				}
			}
		}
		done := make(chan struct{})
		go func() { defer close(done); forward(upstream, downstream) }()
		forward(downstream, upstream)
		downstream.CloseNow()
		<-done
	}), issue(3))
	defer splicer.Close()
	goodClient := client(splicer.Listener.Addr().String(), &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS13})
	defer goodClient.CloseIdleConnections()
	conn, _, err := websocket.Dial(ctx, "wss://"+victimHost+"/v1/feed", &websocket.DialOptions{HTTPClient: goodClient})
	add("two_trusted_tls_legs", err == nil)
	if err != nil {
		return
	}
	defer conn.CloseNow()
	principal, err := replica.RelayClient(ctx, conn, gateway.Fingerprint)
	add("vulnerable_baseline_authentication", err == nil)
	if err != nil {
		return
	}
	wsjson.Write(ctx, conn, contracts.FeedFrame{T: "Hello", Protocol: contracts.FeedProtocol, ReplicaID: principal.ID, SchemaOffer: []int{1}, Credits: contracts.ByteCredit})
	found := false
	for i := 0; i < 8; i++ {
		_, data, err := conn.Read(ctx)
		if err != nil {
			break
		}
		var frame contracts.FeedFrame
		json.Unmarshal(data, &frame)
		if frame.T != "Heartbeat" {
			wsjson.Write(ctx, conn, contracts.FeedFrame{T: "Ack", Credits: len(data)})
		}
		if strings.Contains(string(data), marker) {
			found = true
			break
		}
	}
	add("vulnerable_baseline_splicer_reads_feed", found && observed.Load())
	add("four_auth_frames_forwarded_verbatim", frames.Load() == 4)
	conn.CloseNow()
	for _, control := range []struct {
		name, host string
		config     *tls.Config
	}{
		{"wrong_name", "wrong.relay.test", &tls.Config{RootCAs: roots}},
		{"untrusted_certificate", victimHost, &tls.Config{RootCAs: x509.NewCertPool()}},
		{"direct_pinned_mtls", victimHost, replica.ClientTLS(gateway.Fingerprint)},
	} {
		c := client(splicer.Listener.Addr().String(), control.config)
		socket, _, err := websocket.Dial(ctx, "wss://"+control.host+"/v1/feed", &websocket.DialOptions{HTTPClient: c})
		if socket != nil {
			socket.CloseNow()
		}
		c.CloseIdleConnections()
		add(control.name+"_rejects_splice", err != nil)
	}
}
