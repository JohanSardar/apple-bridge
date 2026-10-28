package relay

import (
	"context"
	"crypto/tls"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"
)

func TestControlSNIOnPublicPort(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	store, err := Open(filepath.Join(t.TempDir(), "state.json"), "relay.test", Limits{Connections: 2, BytesPerSecond: 100000, MonthlyBytes: 1000000})
	if err != nil {
		t.Fatal(err)
	}
	relay := New(store)
	defer relay.Close()
	raw, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer raw.Close()
	control := NewControlListener(ctx)
	defer control.Close()
	relay.ControlHost = "example.com"
	relay.Control = control
	fixture := httptest.NewTLSServer(http.NotFoundHandler())
	defer fixture.Close()
	api := &http.Server{Handler: relay.Handler(""), TLSConfig: fixture.TLS.Clone()}
	defer api.Close()
	go api.Serve(tls.NewListener(control, api.TLSConfig))
	go relay.Serve(ctx, raw)
	transport := fixture.Client().Transport.(*http.Transport).Clone()
	defer transport.CloseIdleConnections()
	transport.DialContext = func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "tcp", raw.Addr().String())
	}
	client := &http.Client{Transport: transport, Timeout: time.Second}
	resp, err := client.Get("https://example.com/challenge")
	if err != nil {
		t.Fatal(err)
	}
	body, err := io.ReadAll(resp.Body)
	resp.Body.Close()
	if err != nil || resp.StatusCode != 200 || len(body) == 0 {
		t.Fatal("control API did not traverse shared public port")
	}
	if resp, err = client.Get("https://unknown.relay.test/challenge"); err == nil {
		resp.Body.Close()
		t.Fatal("unknown SNI reached control API")
	}
}
