package scenarios

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"io"
	"math/big"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/coder/websocket"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func relayStreams(ctx context.Context, lab *relayLab, key ed25519.PrivateKey, c relay.Credential, add func(string, bool)) {
	device, _ := lab.server.Store.Device(c.Hostname)
	device.Limits.Connections = 8
	if err := lab.server.Store.Policy(c.Hostname, device.Limits, false, false); err != nil {
		add("stream_connection_budget", false)
		return
	}
	api := httptest.NewTLSServer(lab.server.Handler(""))
	defer func() { api.Close() }()
	listener := relay.ListenTunnel(ctx, api.URL, api.Client(), key, c)
	defer listener.Close()
	_, private, _ := ed25519.GenerateKey(rand.Reader)
	template := &x509.Certificate{SerialNumber: big.NewInt(1), Subject: pkix.Name{CommonName: c.Hostname}, DNSNames: []string{c.Hostname}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().Add(time.Hour), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth}}
	der, err := x509.CreateCertificate(rand.Reader, template, template, private.Public(), private)
	if err != nil {
		add("certificate", false)
		return
	}
	roots := x509.NewCertPool()
	cert, _ := x509.ParseCertificate(der)
	roots.AddCert(cert)
	secure := tls.NewListener(listener, &tls.Config{Certificates: []tls.Certificate{{Certificate: [][]byte{der}, PrivateKey: private}}, MinVersion: tls.VersionTLS12})
	mux := http.NewServeMux()
	mux.HandleFunc("/sse", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/event-stream")
		for {
			if _, err := io.WriteString(w, "data: bridge\n\n"); err != nil {
				return
			}
			http.NewResponseController(w).Flush()
			select {
			case <-r.Context().Done():
				return
			case <-time.After(100 * time.Millisecond):
			}
		}
	})
	mcpServer := mcp.NewServer(&mcp.Implementation{Name: "relay-lab", Version: "1"}, nil)
	mux.Handle("/mcp", mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return mcpServer }, nil))
	mux.HandleFunc("/ws", func(w http.ResponseWriter, r *http.Request) {
		ws, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer ws.CloseNow()
		kind, b, err := ws.Read(r.Context())
		if err == nil {
			ws.Write(r.Context(), kind, b)
		}
	})
	server := &http.Server{Handler: mux}
	defer server.Close()
	go server.Serve(secure)
	transport := &http.Transport{TLSClientConfig: &tls.Config{RootCAs: roots}, DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", lab.address)
	}}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 15 * time.Minute}
	var resp *http.Response
	until := time.Now().Add(5 * time.Second)
	for time.Now().Before(until) {
		resp, err = client.Get("https://" + c.Hostname + "/sse")
		if err == nil {
			break
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(50 * time.Millisecond):
		}
	}
	if err != nil {
		add("outbound_tunnel_tls", false)
		return
	}
	duration := 10 * time.Minute
	if os.Getenv("AB_LAB_RELAY_STREAM_SECONDS") == "4" {
		duration = 4 * time.Second
	}
	started := time.Now()
	b := make([]byte, 14)
	for time.Since(started) < duration && err == nil {
		_, err = io.ReadFull(resp.Body, b)
	}
	add("sse_survives_idle_budget", err == nil)
	resp.Body.Close()
	ws, _, err := websocket.Dial(ctx, "wss://"+c.Hostname+"/ws", &websocket.DialOptions{HTTPClient: client})
	ok := err == nil
	if ok {
		err = ws.Write(ctx, websocket.MessageText, []byte("echo"))
		_, b, e := ws.Read(ctx)
		ok = err == nil && e == nil && string(b) == "echo"
		ws.CloseNow()
	}
	add("websocket_duplex", ok)
	mcpClient := mcp.NewClient(&mcp.Implementation{Name: "relay-client", Version: "1"}, nil)
	mcpSession, mcpErr := mcpClient.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: "https://" + c.Hostname + "/mcp", HTTPClient: client}, nil)
	ok = mcpErr == nil
	if ok {
		defer mcpSession.Close()
		ok = mcpSession.Ping(ctx, nil) == nil
	}
	add("mcp_streamable_http_session", ok)
	resp, err = client.Get("https://" + c.Hostname + "/sse")
	if err != nil {
		add("restart_active_stream", false)
		return
	}
	started = time.Now()
	lab.server.Close()
	lab.stop()
	lab.listener.Close()
	lab.api.Close()
	<-lab.done
	apiURL, _ := url.Parse(api.URL)
	api.Close()
	rawListener, e := net.Listen("tcp", lab.address)
	if e != nil {
		add("restart_raw_listener", false)
		return
	}
	lab.server = relay.New(lab.server.Store)
	lab.server.IdleTimeout = 2 * time.Second
	lab.listener = rawListener
	lab.done = make(chan error, 1)
	lab.once = sync.Once{}
	restarted, cancel := context.WithCancel(ctx)
	lab.stop = cancel
	lab.api = httptest.NewServer(lab.server.Handler(""))
	go func() { lab.done <- lab.server.Serve(restarted, rawListener) }()
	controlListener, e := net.Listen("tcp", apiURL.Host)
	if e != nil {
		add("restart_control_listener", false)
		return
	}
	api = httptest.NewUnstartedServer(lab.server.Handler(""))
	api.Listener = controlListener
	api.StartTLS()
	resp.Body.Close()
	transport.CloseIdleConnections()
	ok = false
	for time.Since(started) < 5*time.Second {
		resp, err = client.Get("https://" + c.Hostname + "/mcp")
		if err == nil {
			resp.Body.Close()
			ok = true
			break
		}
		select {
		case <-ctx.Done():
			return
		case <-time.After(50 * time.Millisecond):
		}
	}
	add("relay_listener_restart_reconnect_under_5s", ok)
	add("mcp_session_resumes_after_relay_restart", mcpSession != nil && mcpSession.Ping(ctx, nil) == nil)
	p, err := relay.NewProof(ctx, api.Client(), api.URL, key, c, "tunnel")
	add("signed_proof_single_use", err == nil && lab.server.Store.Verify(p, "tunnel") && !lab.server.Store.Verify(p, "tunnel"))
}
