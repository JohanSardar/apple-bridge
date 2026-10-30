package scenarios

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/frontdoor"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"tailscale.com/ipn"
	"tailscale.com/ipn/ipnstate"
	"tailscale.com/tsnet"
)

type privateLabNode struct{ *remoteFlowNode }

func (n privateLabNode) LocalClient() (remote.LocalClient, error) { return n, nil }
func (n privateLabNode) StatusWithoutPeers(ctx context.Context) (*ipnstate.Status, error) {
	st, err := n.remoteFlowNode.StatusWithoutPeers(ctx)
	if st.Self != nil {
		st.Self.DNSName = "bridge.fixture.ts.net."
	}
	return st, err
}
func (n privateLabNode) Listen(string, string) (net.Listener, error) { return n.listener, nil }

type privateConnectNode struct {
	config    *tsnet.Server
	address   string
	persisted *atomic.Bool
}

func (n privateConnectNode) Up(context.Context) (*ipnstate.Status, error) {
	if n.config.Store == nil || !n.config.Ephemeral {
		n.persisted.Store(true)
	} else {
		n.config.Store.WriteState(ipn.StateKey("identity"), []byte("synthetic-node-identity-secret"))
	}
	return &ipnstate.Status{}, nil
}
func (n privateConnectNode) Close() error { return nil }
func (n privateConnectNode) Dial(ctx context.Context, _, _ string) (net.Conn, error) {
	return (&net.Dialer{}).DialContext(ctx, "tcp", n.address)
}

func runPrivateConnect(ctx context.Context, s Scenario, level string) Evidence {
	if level == "replica" || level == "real" {
		return runRemotePrivateReal(ctx, s, level)
	}

	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), RunID: "private-protocol", NetworkPath: "production private handler and connect proxy through isolated node transport"}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	home, err := os.MkdirTemp("", "ab-private-")
	if err != nil {
		add("home", false)
		return e
	}
	defer os.RemoveAll(home)
	for _, expired := range []bool{false, true} {
		func() {
			home := filepath.Join(home, "live")
			if expired {
				home += "-expired"
			}
			ctx, cancel := context.WithTimeout(ctx, 8*time.Second)
			defer cancel()
			listener, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				add("listener", false)
				return
			}
			node := privateLabNode{&remoteFlowNode{listener: listener}}
			var clock atomic.Int64
			clock.Store(time.Now().Unix())
			authority, err := trust.Open(home)
			if err != nil {
				add("authority", false)
				return
			}
			token, err := authority.CreateAPIToken(nil, false)
			if err != nil {
				add("api_token", false)
				return
			}
			api := frontdoor.Handler(frontdoor.Deps{Authority: authority, Address: "127.0.0.1:7780"}, nil)
			r := remote.NewWithOptions(home, remote.Options{PrivateAPI: api, PrivateNode: func(string, string) (remote.PrivateNode, error) { return node, nil }, Now: func() time.Time { return time.Unix(clock.Load(), 0) }})
			done := make(chan struct{})
			go func() {
				defer close(done)
				r.Run(ctx, http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) { http.Error(w, "passkey required", 401) }), frontdoor.Handler(frontdoor.Deps{Authority: authority, Address: "127.0.0.1:7780", PublicHost: r.Host}, nil), http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
					w.Header().Set("Content-Type", "text/event-stream")
					io.WriteString(w, "data: private-feed\n\n")
				}))
			}()
			defer func() { cancel(); <-done }()
			var out remoteFlowOutput
			err = remote.CommandWithOpener(ctx, home, "on", "private", "", &out, func(context.Context, string) error { remoteFilesPrivate(home, add); node.stage.Store(2); return nil })
			add("private_on", err == nil)
			var st remote.Status
			var status bytes.Buffer
			remote.Command(ctx, home, "status", "", "", &status)
			json.Unmarshal(status.Bytes(), &st)
			code := strings.TrimPrefix(st.Message, "bridge connect ")
			raw, _ := base64.RawURLEncoding.DecodeString(code)
			var invite struct{ Secret string }
			json.Unmarshal(raw, &invite)
			transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				return (&net.Dialer{}).DialContext(ctx, "tcp", listener.Addr().String())
			}}
			defer transport.CloseIdleConnections()
			client := &http.Client{Transport: transport}
			if expired {
				clock.Add(301)
			}
			var persisted atomic.Bool
			connectListener, err := net.Listen("tcp", "127.0.0.1:0")
			if err != nil {
				add("connect_listener", false)
				return
			}
			defer connectListener.Close()
			localTransport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
				return (&net.Dialer{}).DialContext(ctx, "tcp", connectListener.Addr().String())
			}}
			defer localTransport.CloseIdleConnections()
			localClient := &http.Client{Transport: privateTokenTransport{localTransport, token}, Timeout: 3 * time.Second}
			connectCtx, stop := context.WithCancel(ctx)
			connected := make(chan error, 1)
			var connectOut remoteFlowOutput
			go func() {
				connected <- remote.ConnectWithOptions(connectCtx, home, code, &connectOut, remote.ConnectOptions{Listener: connectListener, Node: func(config *tsnet.Server) remote.ConnectNode {
					return privateConnectNode{config, listener.Addr().String(), &persisted}
				}})
			}()
			defer stop()
			if expired {
				err = <-connected
				add("expired_invitation_rejected", err != nil && strings.Contains(err.Error(), "invitation rejected"))
				return
			}
			for deadline := time.Now().Add(3 * time.Second); !strings.Contains(connectOut.String(), "http://localhost:7780") && time.Now().Before(deadline); {
				time.Sleep(10 * time.Millisecond)
			}
			req, _ := http.NewRequestWithContext(ctx, "POST", "http://localhost:7780/_bridge/private/invite", nil)
			req.Header.Set("Authorization", "Bearer "+invite.Secret)
			resp, err := client.Do(req)
			add("invitation_reuse_rejected", err == nil && resp.StatusCode == 403)
			if resp != nil {
				resp.Body.Close()
			}
			mcpClient := mcp.NewClient(&mcp.Implementation{Name: "private-client", Version: "1"}, nil)
			session, err := mcpClient.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: "http://localhost:7780/mcp", HTTPClient: localClient}, nil)
			ok := err == nil
			if ok {
				_, err = session.ListTools(ctx, nil)
				ok = err == nil
				session.Close()
			}
			add("private_mcp_tools_list", ok)
			response, err := localClient.Get("http://localhost:7780/v1/feed")
			body := ""
			if err == nil {
				data, _ := io.ReadAll(response.Body)
				response.Body.Close()
				body = string(data)
			}
			add("private_feed_dispatch", err == nil && strings.Contains(body, "private-feed"))
			response, err = localClient.Get("http://localhost:7780/")
			add("private_transport_retains_authentication", err == nil && response.StatusCode == 401)
			if response != nil {
				response.Body.Close()
			}
			stop()
			<-connected
			safe := !persisted.Load()
			filepath.WalkDir(filepath.Join(home, "connect"), func(path string, d os.DirEntry, err error) error {
				if err == nil && !d.IsDir() {
					data, _ := os.ReadFile(path)
					safe = safe && !bytes.Contains(data, []byte("synthetic-node-identity-secret")) && !strings.Contains(d.Name(), "tailscaled.state")
				}
				return nil
			})
			add("connect_has_no_persisted_identity", safe)
			remoteFilesPrivate(home, add)
		}()
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

type privateTokenTransport struct {
	base  http.RoundTripper
	token string
}

func (t privateTokenTransport) RoundTrip(r *http.Request) (*http.Response, error) {
	r = r.Clone(r.Context())
	r.Header.Set("Authorization", "Bearer "+t.token)
	return t.base.RoundTrip(r)
}
