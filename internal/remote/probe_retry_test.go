package remote

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"sync/atomic"
	"testing"
	"time"
)

type startupTransport struct {
	base  http.RoundTripper
	calls atomic.Int32
}

func (t *startupTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	if t.calls.Add(1) == 1 {
		return nil, io.EOF
	}
	return t.base.RoundTrip(r)
}
func TestProbeWaitsForOutboundTunnel(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/mcp" {
			w.Header().Set("WWW-Authenticate", "Bearer")
			w.WriteHeader(401)
			return
		}
		if r.URL.Path == streamCheckPath {
			w.Header().Set("Content-Type", "text/event-stream")
			fmt.Fprint(w, ": bridge remote check\n\n")
			w.(http.Flusher).Flush()
			time.Sleep(5 * time.Millisecond)
			fmt.Fprint(w, ": bridge remote check\n\n")
		}
	}))
	defer server.Close()
	transport := &startupTransport{base: http.DefaultTransport}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	if _, err := ProbeHTTP(ctx, &http.Client{Transport: transport}, server.URL, time.Millisecond); err != nil {
		t.Fatal(err)
	}
	if transport.calls.Load() != 5 {
		t.Fatalf("got %d requests", transport.calls.Load())
	}
}
