package remote

import (
	"context"
	"crypto/rand"
	"crypto/subtle"
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
	"sync"
	"time"

	"tailscale.com/ipn/ipnstate"
	"tailscale.com/ipn/store/mem"
	"tailscale.com/tsnet"
)

type privateCode struct {
	Host   string `json:"host"`
	Secret string `json:"secret"`
}

func (r *Remote) servePrivate(ctx context.Context, cfg Config, status *Status, fail func(error)) {
	factory := r.options.PrivateNode
	if factory == nil {
		factory = func(home, name string) (PrivateNode, error) {
			node, err := embeddedNode(home, name, func(string, ...any) {})
			return embeddedServer{node}, err
		}
	}
	node, err := factory(r.home, cfg.Name)
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
		st, err := client.StatusWithoutPeers(ctx)
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
		status.PendingLink = "login pending"
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
	secret := make([]byte, 32)
	if _, err := rand.Read(secret); err != nil {
		fail(err)
		return
	}
	invitation := base64.RawURLEncoding.EncodeToString(secret)
	now := r.options.Now
	if now == nil {
		now = time.Now
	}
	expires := now().Add(5 * time.Minute)
	var mu sync.Mutex
	consumed := false
	session := ""
	code, _ := json.Marshal(privateCode{Host: host, Secret: invitation})
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
		mu.Lock()
		if req.URL.Path == "/_bridge/private/invite" {
			if req.Method != "POST" || consumed || !now().Before(expires) || subtle.ConstantTimeCompare([]byte(req.Header.Get("Authorization")), []byte("Bearer "+invitation)) != 1 {
				mu.Unlock()
				http.Error(w, "invitation rejected", 403)
				return
			}
			token := make([]byte, 32)
			if _, err := rand.Read(token); err != nil {
				mu.Unlock()
				http.Error(w, "unavailable", 503)
				return
			}
			session = base64.RawURLEncoding.EncodeToString(token)
			consumed = true
			mu.Unlock()
			json.NewEncoder(w).Encode(map[string]string{"session": session})
			return
		}
		allowed := session != "" && subtle.ConstantTimeCompare([]byte(req.Header.Get("X-Bridge-Private-Session")), []byte(session)) == 1
		mu.Unlock()
		if !allowed {
			http.Error(w, "private connection required", 403)
			return
		}
		req.Header.Del("X-Bridge-Private-Session")
		req.Header.Del("Forwarded")
		req.Header.Del("X-Forwarded-Host")
		req.Header.Del("X-Forwarded-Proto")
		req.Header.Del("X-Forwarded-For")
		web, api := r.options.PrivateWeb, r.options.PrivateAPI
		if web == nil {
			web = r.web
		}
		if api == nil {
			api = r.api
		}
		r.dispatch(w, req, web, api)
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

type ConnectNode interface {
	Up(context.Context) (*ipnstate.Status, error)
	Dial(context.Context, string, string) (net.Conn, error)
	Close() error
}
type ConnectOptions struct {
	Listener net.Listener
	Node     func(*tsnet.Server) ConnectNode
}

func Connect(ctx context.Context, home, code string, out io.Writer) error {
	return ConnectWithOptions(ctx, home, code, out, ConnectOptions{})
}
func ConnectWithOptions(ctx context.Context, home, code string, out io.Writer, options ConnectOptions) error {
	raw, err := base64.RawURLEncoding.DecodeString(code)
	if err != nil || len(raw) > 4096 {
		return fmt.Errorf("invalid private connection code")
	}
	var target privateCode
	if json.Unmarshal(raw, &target) != nil || len(target.Secret) != 43 || !strings.HasSuffix(target.Host, ".ts.net") || strings.ContainsAny(target.Host, "/:@?# ") {
		return fmt.Errorf("invalid private connection host")
	}
	listener := options.Listener
	if listener == nil {
		listener, err = net.Listen("tcp", "127.0.0.1:7780")
	}
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
	config := &tsnet.Server{Store: new(mem.Store), Ephemeral: true, Dir: dir, Hostname: "bridge-connect", UserLogf: func(format string, args ...any) { fmt.Fprintf(out, format+"\n", args...) }, Logf: func(string, ...any) {}}
	var node ConnectNode = config
	if options.Node != nil {
		node = options.Node(config)
	}
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
	client := &http.Client{Transport: transport, Timeout: 15 * time.Second}
	redeem, err := http.NewRequestWithContext(ctx, "POST", upstream.String()+"/_bridge/private/invite", nil)
	if err != nil {
		return err
	}
	redeem.Host = "localhost:7780"
	redeem.Header.Set("Authorization", "Bearer "+target.Secret)
	response, err := client.Do(redeem)
	if err != nil {
		return fmt.Errorf("private invitation unavailable")
	}
	var grant struct {
		Session string `json:"session"`
	}
	err = json.NewDecoder(io.LimitReader(response.Body, 4096)).Decode(&grant)
	response.Body.Close()
	if response.StatusCode != 200 || err != nil || grant.Session == "" {
		return fmt.Errorf("private invitation rejected")
	}

	server := &http.Server{BaseContext: func(net.Listener) context.Context { return ctx }, ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Host != "localhost:7780" {
			http.Error(w, "invalid local host", 403)
			return
		}
		r.Header.Del("Forwarded")
		r.Header.Del("X-Forwarded-Host")
		r.Header.Del("X-Forwarded-Proto")
		r.Header.Del("X-Forwarded-For")
		r.Header.Set("X-Bridge-Private-Session", grant.Session)
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
