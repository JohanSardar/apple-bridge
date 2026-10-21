package remote

import (
	"context"
	"crypto/tls"
	"net"
	"net/http"
	"net/netip"
	"strings"
	"time"
)

func (r *Remote) handler(proxyPath string) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		host := r.Host()
		if host == "" {
			http.Error(w, "remote disabled", 503)
			return
		}
		if proxyPath != "" {
			ip, _, err := net.SplitHostPort(req.RemoteAddr)
			if err != nil || !net.ParseIP(ip).IsLoopback() {
				http.Error(w, "unauthorized proxy", 403)
				return
			}
			if req.Host != host && req.Host != strings.TrimSuffix(host, ":443") {
				http.Error(w, "invalid public host", 403)
				return
			}
			if req.URL.Path != proxyPath && !strings.HasPrefix(req.URL.Path, proxyPath+"/") {
				http.Error(w, "unauthorized proxy", 403)
				return
			}
			hosts := req.Header.Values("X-Forwarded-Host")
			protocols := req.Header.Values("X-Forwarded-Proto")
			sources := req.Header.Values("X-Forwarded-For")
			if len(hosts) != 1 || hosts[0] != host || len(protocols) != 1 || protocols[0] != "https" || len(sources) != 1 {
				http.Error(w, "invalid proxy headers", 403)
				return
			}
			source, err := netip.ParseAddr(sources[0])
			if err != nil {
				http.Error(w, "invalid proxy source", 403)
				return
			}
			req = req.Clone(req.Context())
			req.URL.Path = strings.TrimPrefix(req.URL.Path, proxyPath)
			if req.URL.Path == "" {
				req.URL.Path = "/"
			}
			if req.URL.RawPath != "" {
				req.URL.RawPath = strings.TrimPrefix(req.URL.RawPath, proxyPath)
			}
			req.RequestURI = req.URL.RequestURI()
			req.RemoteAddr = net.JoinHostPort(source.String(), "0")
			req.TLS = &tls.ConnectionState{}
		} else if req.Host != host {
			http.Error(w, "invalid public host", 403)
			return
		}
		req.Host = host
		req.Header.Del("X-Forwarded-Host")
		req.Header.Del("X-Forwarded-Proto")
		req.Header.Del("X-Forwarded-For")
		if proxyPath == "" {
			if source, ok := req.Context().Value(clientAddressKey{}).(string); ok {
				req = req.Clone(req.Context())
				req.RemoteAddr = source
			}
		}
		if req.URL.Path == "/v1/feed" && r.feed != nil {
			r.feed.ServeHTTP(w, req)
			return
		}
		if req.URL.Path == "/v1/feed" || strings.HasPrefix(req.URL.Path, "/v1/feed/") {
			http.NotFound(w, req)
			return
		}
		if strings.HasPrefix(req.URL.Path, "/v1/") || req.URL.Path == "/mcp" || strings.HasPrefix(req.URL.Path, "/mcp/") {
			r.api.ServeHTTP(w, req)
			return
		}
		r.web.ServeHTTP(w, req)
	})
}

func (r *Remote) serveListener(ctx context.Context, listener net.Listener, proxyPath string, fail func(error)) {
	server := &http.Server{BaseContext: func(net.Listener) context.Context { return ctx }, Handler: r.handler(proxyPath), ConnContext: clientConnection, ReadHeaderTimeout: 10 * time.Second, IdleTimeout: 30 * time.Second}
	defer server.Close()
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	select {
	case <-ctx.Done():
	case err := <-done:
		fail(err)
	}
}
