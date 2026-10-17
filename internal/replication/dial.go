package replication

import (
	"context"
	"encoding/json"
	"net/http"
	"sort"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
)

type commandSession struct {
	conn    *websocket.Conn
	ctx     context.Context
	mu      sync.Mutex
	pending map[string]chan contracts.CommandResponse
}

func (s *commandSession) deliver(id string, response contracts.CommandResponse) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if ch := s.pending[id]; ch != nil {
		select {
		case ch <- response:
		default:
		}
	}
}
func (c *Client) Exchange(ctx context.Context, delegation string, raw json.RawMessage) (contracts.CommandResponse, error) {
	var empty contracts.CommandResponse
	c.mu.Lock()
	session := c.session
	c.mu.Unlock()
	if session == nil {
		return empty, contracts.ErrUnavailable
	}
	session.mu.Lock()
	if len(session.pending) >= 16 {
		session.mu.Unlock()
		return empty, contracts.ErrUnavailable
	}
	id := contracts.NewID("request")
	response := make(chan contracts.CommandResponse, 1)
	session.pending[id] = response
	session.mu.Unlock()
	defer func() { session.mu.Lock(); delete(session.pending, id); session.mu.Unlock() }()
	bounded, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	if _, err := send(bounded, session.conn, contracts.FeedFrame{T: "CommandRequest", ID: id, Delegation: delegation, Request: raw}); err != nil {
		return empty, contracts.ErrUnavailable
	}
	select {
	case result := <-response:
		return result, nil
	case <-bounded.Done():
		return empty, contracts.ErrUnavailable
	case <-session.ctx.Done():
		return empty, contracts.ErrUnavailable
	}
}

func (c *Client) Handler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		e, err := c.Authority.Enrollment()
		if err != nil || e == nil || e.Dial == "" || r.TLS == nil || len(r.TLS.PeerCertificates) != 1 || trust.Fingerprint(r.TLS.PeerCertificates[0].Raw) != e.Fingerprint || r.Header.Get("Origin") != "" {
			http.Error(w, "unauthorized", 401)
			return
		}
		if !c.connectionMu.TryLock() {
			http.Error(w, "busy", 409)
			return
		}
		defer c.connectionMu.Unlock()
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		c.consume(r.Context(), conn, *e)
	})
}

func (s *Server) Dial(ctx context.Context) {
	active := map[string]bool{}
	done := make(chan string, 32)
	tick := time.NewTicker(250 * time.Millisecond)
	defer tick.Stop()
	for {
		select {
		case <-ctx.Done():
			return
		case id := <-done:
			delete(active, id)
		case <-tick.C:
			replicas, err := s.Authority.DialReplicas()
			if err != nil {
				continue
			}
			for _, p := range replicas {
				if active[p.ID] {
					continue
				}
				active[p.ID] = true
				go func(p trust.Principal) {
					s.dialReplica(ctx, p)
					select {
					case done <- p.ID:
					case <-ctx.Done():
					}
				}(p)
			}
		}
	}
}
func (s *Server) dialReplica(ctx context.Context, p trust.Principal) {
	ctx, cancel := s.Authority.WatchPrincipal(ctx, p)
	defer cancel()
	defer s.updateDial(p.ID, func(status *contracts.DialStatus) { status.State = "revoked" })
	backoff := 250 * time.Millisecond
	for ctx.Err() == nil {
		s.updateDial(p.ID, func(status *contracts.DialStatus) { status.State = "connecting"; status.Attempts++ })
		transport := &http.Transport{TLSClientConfig: s.Authority.ClientTLS(p.Fingerprint)}
		bounded, stop := context.WithTimeout(ctx, 5*time.Second)
		started := time.Now()
		conn, response, err := websocket.Dial(bounded, "wss://"+p.Dial+"/v1/feed", &websocket.DialOptions{HTTPClient: &http.Client{Transport: transport}})
		stop()
		if err == nil {
			cert := response.TLS.PeerCertificates[0]
			current, authErr := s.Authority.AuthenticateCertificate(cert)
			if authErr == nil && current.ID == p.ID {
				s.serve(ctx, conn, current, cert)
			}
			conn.CloseNow()
		}
		transport.CloseIdleConnections()
		s.updateDial(p.ID, func(status *contracts.DialStatus) { status.State = "disconnected" })
		if time.Since(started) > 15*time.Second {
			backoff = 250 * time.Millisecond
		}
		timer := time.NewTimer(backoff)
		select {
		case <-ctx.Done():
			timer.Stop()
			return
		case <-timer.C:
		}
		backoff = min(5*time.Second, 2*backoff)
	}
}

func (s *Server) updateDial(id string, fn func(*contracts.DialStatus)) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.statuses == nil {
		s.statuses = map[string]contracts.DialStatus{}
	}
	status := s.statuses[id]
	status.ReplicaID = id
	fn(&status)
	s.statuses[id] = status
}
func (s *Server) DialStatus() []contracts.DialStatus {
	s.mu.Lock()
	defer s.mu.Unlock()
	result := []contracts.DialStatus{}
	for _, status := range s.statuses {
		result = append(result, status)
	}
	sort.Slice(result, func(i, j int) bool { return result[i].ReplicaID < result[j].ReplicaID })
	return result
}
