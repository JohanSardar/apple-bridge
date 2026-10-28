package remote

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httputil"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"tailscale.com/tsnet"
)

type privateCode struct {
	Host string `json:"host"`
}

func (r *Remote) servePrivate(ctx context.Context, cfg Config, status *Status, fail func(error)) {
	node, err := embeddedNode(r.home, cfg.Name, func(string, ...any) {})
	if err != nil {
		fail(err)
		return
	}
	defer node.Close()
	stop := context.AfterFunc(ctx, func() { node.Close() })
	defer stop()
	client, err := node.LocalClient()
	if err != nil {
		fail(err)
		return
	}
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	var host string
	for {
		st, err := client.Status(ctx)
		if err != nil {
			fail(err)
			return
		}
		if st.BackendState == "Running" && st.Self != nil {
			host = strings.TrimSuffix(st.Self.DNSName, ".")
			break
		}
		status.State = "pending"
		status.Message = "Authorize the private Tailscale node"
		status.PendingLink = st.AuthURL
		r.publish(*status)
		r.approval(status, st.AuthURL)
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
	listener, err := node.Listen("tcp", ":7780")
	if err != nil {
		fail(err)
		return
	}
	defer listener.Close()
	code, _ := json.Marshal(privateCode{Host: host})
	status.State = "on"
	status.URL = ""
	status.PendingLink = ""
	status.TLSTermination = "Tailscale encrypted private transport"
	status.Message = "bridge connect " + base64.RawURLEncoding.EncodeToString(code)
	r.publish(*status)
	server := &http.Server{BaseContext: func(net.Listener) context.Context { return ctx }, ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		if req.Host != "localhost:7780" {
			http.Error(w, "invalid private origin", 403)
			return
		}
		req.Header.Del("X-Forwarded-Host")
		req.Header.Del("X-Forwarded-Proto")
		req.Header.Del("X-Forwarded-For")
		r.web.ServeHTTP(w, req)
	})}
	defer server.Close()
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	select {
	case <-ctx.Done():
	case err := <-done:
		fail(err)
	}
}
func Connect(ctx context.Context, home, code string, out io.Writer) error {
	raw, err := base64.RawURLEncoding.DecodeString(code)
	if err != nil || len(raw) > 4096 {
		return fmt.Errorf("invalid private connection code")
	}
	var target privateCode
	if json.Unmarshal(raw, &target) != nil || !strings.HasSuffix(target.Host, ".ts.net") || strings.ContainsAny(target.Host, "/:@?# ") {
		return fmt.Errorf("invalid private connection host")
	}
	listener, err := net.Listen("tcp", "127.0.0.1:7780")
	if err != nil {
		return err
	}
	defer listener.Close()
	dir := filepath.Join(home, "connect")
	if err = os.MkdirAll(dir, 0700); err != nil {
		return err
	}
	if err = os.Chmod(dir, 0700); err != nil {
		return err
	}
	node := &tsnet.Server{Dir: dir, Hostname: "bridge-connect", UserLogf: func(format string, args ...any) { fmt.Fprintf(out, format+"\n", args...) }, Logf: func(string, ...any) {}}
	defer node.Close()
	stop := context.AfterFunc(ctx, func() { node.Close() })
	defer stop()
	if _, err = node.Up(ctx); err != nil {
		return err
	}
	upstream := &url.URL{Scheme: "http", Host: net.JoinHostPort(target.Host, "7780")}
	proxy := httputil.NewSingleHostReverseProxy(upstream)
	transport := &http.Transport{DialContext: node.Dial}
	defer transport.CloseIdleConnections()
	proxy.Transport = transport
	server := &http.Server{BaseContext: func(net.Listener) context.Context { return ctx }, ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != "localhost:7780" {
			http.Error(w, "invalid local host", 403)
			return
		}
		r.Header.Del("Forwarded")
		r.Header.Del("X-Forwarded-Host")
		r.Header.Del("X-Forwarded-Proto")
		r.Header.Del("X-Forwarded-For")
		proxy.ServeHTTP(w, r)
	})}
	defer server.Close()
	fmt.Fprintln(out, "http://localhost:7780")
	done := make(chan error, 1)
	go func() { done <- server.Serve(listener) }()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case err := <-done:
		return err
	}
}
