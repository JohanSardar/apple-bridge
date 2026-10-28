package relay

import (
	"context"
	"crypto/ed25519"
	"encoding/base64"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/xtaci/smux"
)

type Proof struct {
	Hostname   string `json:"hostname"`
	Credential string `json:"credential"`
	Nonce      string `json:"nonce"`
	Signature  string `json:"signature"`
}

func ProofMessage(p Proof, action string) []byte {
	return []byte("apple-bridge-relay/1\n" + action + "\n" + p.Hostname + "\n" + p.Credential + "\n" + p.Nonce)
}

func (s *Store) Verify(p Proof, action string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	expiry, ok := s.challenges[p.Nonce]
	delete(s.challenges, p.Nonce)
	d, exists := s.devices[p.Hostname]
	key, _ := base64.RawURLEncoding.DecodeString(d.PublicKey)
	sig, _ := base64.RawURLEncoding.DecodeString(p.Signature)
	return ok && exists && time.Now().Before(expiry) && !d.Disabled && !d.Revoked && d.CredentialHash == digest(p.Credential) && len(key) == ed25519.PublicKeySize && ed25519.Verify(key, ProofMessage(p, action), sig)
}

func muxConfig() *smux.Config {
	c := smux.DefaultConfig()
	c.Version = 2
	c.KeepAliveInterval = time.Second
	c.KeepAliveTimeout = 3 * time.Second
	c.MaxReceiveBuffer = 4 << 20
	c.MaxStreamBuffer = 256 << 10
	return c
}

type muxTunnel struct{ session *smux.Session }

func (t muxTunnel) Open(ctx context.Context) (io.ReadWriteCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	stop := context.AfterFunc(ctx, func() { t.session.Close() })
	defer stop()
	return t.session.OpenStream()
}

func (s *Server) tunnel(w http.ResponseWriter, r *http.Request) {
	var p Proof
	raw, err := base64.RawURLEncoding.DecodeString(r.Header.Get("X-Bridge-Proof"))
	if err != nil || len(raw) > 4096 || json.Unmarshal(raw, &p) != nil || !s.Store.Verify(p, "tunnel") {
		http.Error(w, "denied", 403)
		return
	}
	ws, err := websocket.Accept(w, r, nil)
	if err != nil {
		return
	}
	defer ws.CloseNow()
	conn := websocket.NetConn(r.Context(), ws, websocket.MessageBinary)
	session, err := smux.Server(conn, muxConfig())
	if err != nil {
		return
	}
	defer session.Close()
	detach, err := s.Attach(p.Hostname, p.Credential, muxTunnel{session})
	if err != nil {
		return
	}
	defer detach()
	s.mu.Lock()
	active := s.sessions[p.Hostname]
	s.mu.Unlock()
	if active == nil {
		return
	}
	select {
	case <-session.CloseChan():
	case <-r.Context().Done():
	case <-active.ctx.Done():
	}
}

type TunnelListener struct {
	ctx      context.Context
	cancel   context.CancelFunc
	incoming chan net.Conn
	done     chan struct{}
	once     sync.Once
}

func ListenTunnel(ctx context.Context, endpoint string, client *http.Client, key ed25519.PrivateKey, credential Credential) *TunnelListener {
	ctx, cancel := context.WithCancel(ctx)
	l := &TunnelListener{ctx: ctx, cancel: cancel, incoming: make(chan net.Conn), done: make(chan struct{})}
	go func() {
		defer close(l.done)
		delay := 100 * time.Millisecond
		for ctx.Err() == nil {
			started := time.Now()
			l.connect(endpoint, client, key, credential)
			if time.Since(started) > 3*time.Second {
				delay = 100 * time.Millisecond
			}
			timer := time.NewTimer(delay)
			select {
			case <-ctx.Done():
				timer.Stop()
				return
			case <-timer.C:
			}
			if delay < time.Second {
				delay *= 2
			}
		}
	}()
	return l
}
func (l *TunnelListener) connect(endpoint string, client *http.Client, key ed25519.PrivateKey, credential Credential) {
	bounded, cancel := context.WithTimeout(l.ctx, 5*time.Second)
	p, err := NewProof(bounded, client, endpoint, key, credential, "tunnel")
	cancel()
	if err != nil {
		return
	}
	raw, _ := json.Marshal(p)
	handshake, finish := context.WithTimeout(l.ctx, 5*time.Second)
	ws, resp, err := websocket.Dial(handshake, endpoint+"/tunnel", &websocket.DialOptions{HTTPClient: client, HTTPHeader: http.Header{"X-Bridge-Proof": {base64.RawURLEncoding.EncodeToString(raw)}}})
	finish()
	if resp != nil && err != nil {
		resp.Body.Close()
	}
	if err != nil {
		return
	}
	defer ws.CloseNow()
	session, err := smux.Client(websocket.NetConn(l.ctx, ws, websocket.MessageBinary), muxConfig())
	if err != nil {
		return
	}
	defer session.Close()
	stop := context.AfterFunc(l.ctx, func() { session.Close() })
	defer stop()
	for {
		stream, err := session.AcceptStream()
		if err != nil {
			return
		}
		select {
		case l.incoming <- stream:
		case <-l.ctx.Done():
			stream.Close()
			return
		}
	}
}
func NewProof(ctx context.Context, client *http.Client, endpoint string, key ed25519.PrivateKey, credential Credential, action string) (Proof, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", endpoint+"/challenge", nil)
	if err != nil {
		return Proof{}, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return Proof{}, err
	}
	defer resp.Body.Close()
	var challenge struct {
		Nonce string `json:"nonce"`
	}
	if resp.StatusCode != 200 {
		return Proof{}, ErrDenied
	}
	if err = json.NewDecoder(io.LimitReader(resp.Body, 4096)).Decode(&challenge); err != nil {
		return Proof{}, err
	}
	p := Proof{Hostname: credential.Hostname, Credential: credential.Credential, Nonce: challenge.Nonce}
	p.Signature = base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, ProofMessage(p, action)))
	return p, nil
}
func (l *TunnelListener) Accept() (net.Conn, error) {
	select {
	case c := <-l.incoming:
		return c, nil
	case <-l.ctx.Done():
		return nil, net.ErrClosed
	}
}
func (l *TunnelListener) Close() error   { l.once.Do(func() { l.cancel(); <-l.done }); return nil }
func (l *TunnelListener) Addr() net.Addr { return tunnelAddr("relay") }

type tunnelAddr string

func (a tunnelAddr) Network() string { return "relay" }
func (a tunnelAddr) String() string  { return string(a) }
