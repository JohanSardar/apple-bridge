package relayfixture

import (
	"bufio"
	"bytes"
	"context"
	"crypto/tls"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"net/http/httputil"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	goRelay "github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/remote"
)

type Limits = goRelay.Limits
type Device = goRelay.Device
type Proof = goRelay.Proof
type Credential = goRelay.Credential
type Enrollment = goRelay.Enrollment
type Tunnel = goRelay.Tunnel
type DNSProvider = goRelay.DNSProvider

var EnrollmentMessage = goRelay.EnrollmentMessage
var ListenTunnel = goRelay.ListenTunnel
var NewProof = goRelay.NewProof
var MissingRecords = goRelay.MissingRecords

type callbackEntry struct {
	path   string
	server *httptest.Server
}

var processes struct {
	sync.Mutex
	items     []*exec.Cmd
	callbacks []callbackEntry
}

func Cleanup(root string) {
	within := func(path string) bool { return path == root || strings.HasPrefix(path, root+string(os.PathSeparator)) }
	processes.Lock()
	var items []*exec.Cmd
	var callbacks []callbackEntry
	kept := processes.items[:0]
	for _, cmd := range processes.items {
		if within(cmd.Args[1]) {
			items = append(items, cmd)
		} else {
			kept = append(kept, cmd)
		}
	}
	processes.items = kept
	remaining := processes.callbacks[:0]
	for _, entry := range processes.callbacks {
		if within(entry.path) {
			callbacks = append(callbacks, entry)
		} else {
			remaining = append(remaining, entry)
		}
	}
	processes.callbacks = remaining
	processes.Unlock()
	for _, cmd := range items {
		cmd.Process.Kill()
		cmd.Wait()
	}
	for _, entry := range callbacks {
		entry.server.Close()
	}
}

type Store struct {
	goStore  *goRelay.Store
	endpoint string
	path     string
}

func Open(path, zone string, limits Limits) (*Store, error) {
	binary := os.Getenv("AB_RELAY_FIXTURE")
	if binary == "" {
		s, e := goRelay.Open(path, zone, limits)
		return &Store{goStore: s, path: path}, e
	}
	raw, _ := json.Marshal(limits)
	cmd := exec.Command(binary, path, zone, string(raw))
	out, e := cmd.StdoutPipe()
	if e != nil {
		return nil, e
	}
	cmd.Stderr = os.Stderr
	if e = cmd.Start(); e != nil {
		return nil, e
	}
	line, e := bufio.NewReader(out).ReadString('\n')
	if e != nil {
		cmd.Process.Kill()
		cmd.Wait()
		return nil, e
	}
	processes.Lock()
	processes.items = append(processes.items, cmd)
	processes.Unlock()
	return &Store{endpoint: string(bytes.TrimSpace([]byte(line))), path: path}, nil
}
func (s *Store) rpc(action string, input map[string]any, output any) error {
	if input == nil {
		input = map[string]any{}
	}
	input["action"] = action
	raw, e := json.Marshal(input)
	if e != nil {
		return e
	}
	client := &http.Client{Timeout: 30 * time.Second}
	response, e := client.Post(s.endpoint+"/rpc", "application/json", bytes.NewReader(raw))
	if e != nil {
		return e
	}
	defer response.Body.Close()
	var result struct {
		OK    bool            `json:"ok"`
		Value json.RawMessage `json:"value"`
		Error string          `json:"error"`
	}
	if e = json.NewDecoder(response.Body).Decode(&result); e != nil {
		return e
	}
	if !result.OK {
		return fmt.Errorf("Rust fixture: %s", result.Error)
	}
	if output != nil {
		return json.Unmarshal(result.Value, output)
	}
	return nil
}
func (s *Store) RequireOwnerKeys(path string) {
	if s.goStore != nil {
		s.goStore.RequireOwnerKeys(path)
		return
	}
	s.rpc("owner", map[string]any{"path": path}, nil)
}
func (s *Store) ValidateOwnerKeys() error {
	if s.goStore != nil {
		return s.goStore.ValidateOwnerKeys()
	}
	return s.rpc("validate", nil, nil)
}
func (s *Store) Device(host string) (Device, bool) {
	if s.goStore != nil {
		return s.goStore.Device(host)
	}
	var d *Device
	if s.rpc("device", map[string]any{"host": host}, &d) != nil || d == nil {
		return Device{}, false
	}
	return *d, true
}
func (s *Store) Policy(host string, l Limits, disabled, revoked bool) error {
	if s.goStore != nil {
		return s.goStore.Policy(host, l, disabled, revoked)
	}
	return s.rpc("policy", map[string]any{"host": host, "limits": l, "disabled": disabled, "revoked": revoked}, nil)
}
func (s *Store) Verify(p Proof, action string) bool {
	if s.goStore != nil {
		return s.goStore.Verify(p, action)
	}
	var ok bool
	return s.rpc("verify", map[string]any{"proof": p, "proof_action": action}, &ok) == nil && ok
}

type Server struct {
	*goRelay.Server
	Store    *Store
	goServer *goRelay.Server
	id       int
	callback *httptest.Server
	streams  sync.Map
}

func New(store *Store) *Server {
	s := &Server{Store: store, Server: goRelay.New(store.goStore)}
	if store.goStore != nil {
		s.goServer = s.Server
		return s
	}
	s.callback = httptest.NewServer(http.HandlerFunc(s.call))
	processes.Lock()
	processes.callbacks = append(processes.callbacks, callbackEntry{store.path, s.callback})
	processes.Unlock()
	if e := store.rpc("new", map[string]any{"callback": s.callback.URL, "admin": "synthetic-admin"}, &s.id); e != nil {
		panic(e)
	}
	return s
}
func (s *Server) rpc(action string, input map[string]any, output any) error {
	if input == nil {
		input = map[string]any{}
	}
	input["id"] = s.id
	return s.Store.rpc(action, input, output)
}
func (s *Server) Handler(admin string) http.Handler {
	if s.goServer != nil {
		return s.goServer.Handler(admin)
	}
	var id int
	if e := s.rpc("handler", map[string]any{"admin": admin}, &id); e != nil {
		panic(e)
	}
	target, _ := url.Parse(fmt.Sprintf("%s/api/%d", s.Store.endpoint, id))
	return httputil.NewSingleHostReverseProxy(target)
}
func (s *Server) call(w http.ResponseWriter, r *http.Request) {
	switch r.URL.Path {
	case "/cancel":
		var in struct{ Key string }
		json.NewDecoder(r.Body).Decode(&in)
		if cancel, ok := s.streams.Load(in.Key); ok {
			cancel.(context.CancelFunc)()
		}
		w.WriteHeader(204)
	case "/dns":
		var in struct{ Name, Value string }
		if json.NewDecoder(r.Body).Decode(&in) != nil || s.DNS == nil || s.DNS.TXT(r.Context(), in.Name, in.Value) != nil {
			w.WriteHeader(503)
			return
		}
		w.WriteHeader(204)
	case "/cname":
		resolver := s.Resolver
		if resolver == nil {
			resolver = net.DefaultResolver
		}
		name, e := resolver.LookupCNAME(r.Context(), r.URL.Query().Get("name"))
		if e != nil {
			w.WriteHeader(503)
			return
		}
		json.NewEncoder(w).Encode(name)
	case "/ct":
		if s.CTClient == nil {
			w.WriteHeader(503)
			return
		}
		u := "https://crt.sh/?" + r.URL.RawQuery
		request, _ := http.NewRequestWithContext(r.Context(), "GET", u, nil)
		response, e := s.CTClient.Do(request)
		if e != nil {
			w.WriteHeader(503)
			return
		}
		defer response.Body.Close()
		w.WriteHeader(response.StatusCode)
		io.Copy(w, response.Body)
	default:
		w.WriteHeader(404)
	}
}
func (s *Server) Attach(host, token string, tunnel Tunnel) (func(), error) {
	if s.goServer != nil {
		return s.goServer.Attach(host, token, tunnel)
	}
	listener, e := net.Listen("tcp", "127.0.0.1:0")
	if e != nil {
		return nil, e
	}
	ctx, cancel := context.WithCancel(context.Background())
	if e = s.rpc("attach", map[string]any{"host": host, "token": token, "address": listener.Addr().String(), "callback": s.callback.URL}, nil); e != nil {
		cancel()
		listener.Close()
		return nil, e
	}
	go func() {
		defer listener.Close()
		for {
			socket, e := listener.Accept()
			if e != nil {
				return
			}
			go func() {
				defer socket.Close()
				ctx, cancelStream := context.WithCancel(ctx)
				defer cancelStream()
				key := socket.RemoteAddr().String()
				s.streams.Store(key, cancelStream)
				defer s.streams.Delete(key)
				up, e := tunnel.Open(ctx)
				if e != nil {
					return
				}
				defer up.Close()
				stop := context.AfterFunc(ctx, func() { socket.Close(); up.Close() })
				defer stop()
				done := make(chan struct{})
				go func() { io.Copy(up, socket); up.Close(); socket.Close(); close(done) }()
				io.Copy(socket, up)
				socket.Close()
				up.Close()
				<-done
			}()
		}
	}()
	return func() { cancel(); listener.Close(); s.Disconnect(host) }, nil
}
func (s *Server) Disconnect(host string) {
	if s.goServer != nil {
		s.goServer.Disconnect(host)
		return
	}
	s.rpc("disconnect", map[string]any{"host": host}, nil)
}
func (s *Server) Close() {
	if s.goServer != nil {
		s.goServer.Close()
		return
	}
	s.rpc("close", nil, nil)
	s.streams.Range(func(_, value any) bool { value.(context.CancelFunc)(); return true })
	if s.callback != nil {
		s.callback.Close()
	}
}
func (s *Server) MonitorCT(ctx context.Context) {
	if s.goServer != nil {
		s.goServer.MonitorCT(ctx)
		return
	}
	s.rpc("monitor", nil, nil)
}
func (s *Server) Serve(ctx context.Context, l net.Listener) error {
	if s.goServer != nil {
		return s.goServer.Serve(ctx, l)
	}
	var address string
	if e := s.rpc("listen", nil, &address); e != nil {
		return e
	}
	defer s.rpc("stop", nil, nil)
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()
	stop := context.AfterFunc(ctx, func() { l.Close() })
	defer stop()
	var workers sync.WaitGroup
	defer workers.Wait()
	for {
		socket, e := l.Accept()
		if e != nil {
			cancel()
			return e
		}
		workers.Add(1)
		go func() {
			defer workers.Done()
			defer socket.Close()
			up, e := (&net.Dialer{}).DialContext(ctx, "tcp", address)
			if e != nil {
				return
			}
			defer up.Close()
			stop := context.AfterFunc(ctx, func() { socket.Close(); up.Close() })
			defer stop()
			done := make(chan struct{})
			go func() { io.Copy(up, socket); up.Close(); socket.Close(); close(done) }()
			io.Copy(socket, up)
			socket.Close()
			up.Close()
			<-done
		}()
	}
}

func (s *Server) ControlTLSConfig(ctx context.Context, home, host string, dns DNSProvider, options remote.CertificateOptions, root string) (*tls.Config, error) {
	if s.goServer != nil {
		return remote.ControlTLSConfig(ctx, home, host, dns, options)
	}
	path := filepath.Join(home, "remote")
	if e := s.rpc("control", map[string]any{"path": path, "host": host, "directory": options.DirectoryURL, "root": root}, nil); e != nil {
		return nil, e
	}
	cert, e := tls.LoadX509KeyPair(filepath.Join(path, "relay-cert.pem"), filepath.Join(path, "relay-key.pem"))
	if e != nil {
		return nil, e
	}
	return &tls.Config{MinVersion: tls.VersionTLS12, GetCertificate: func(*tls.ClientHelloInfo) (*tls.Certificate, error) { return &cert, nil }}, nil
}
