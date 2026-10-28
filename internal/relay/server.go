package relay

import (
	"context"
	"io"
	"net"
	"net/http"
	"sync"
	"time"
)

type Tunnel interface {
	Open(context.Context) (io.ReadWriteCloser, error)
}
type session struct {
	drained sync.WaitGroup
	tunnel  Tunnel
	token   string
	cancel  context.CancelFunc
	ctx     context.Context
	active  int
	next    time.Time
}
type Server struct {
	CTClient    *http.Client
	Resolver    *net.Resolver
	ControlHost string
	Control     *ControlListener
	ctWarnings  map[string]string
	DNS         DNSProvider
	IdleTimeout time.Duration
	Store       *Store
	mu          sync.Mutex
	sessions    map[string]*session
}

func New(store *Store) *Server {
	return &Server{IdleTimeout: 2 * time.Minute, Store: store, ctWarnings: map[string]string{}, sessions: map[string]*session{}}
}
func (s *Server) Attach(host, credential string, tunnel Tunnel) (func(), error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if tunnel == nil || !s.Store.authenticated(host, credential) {
		return nil, ErrDenied
	}
	if old := s.sessions[host]; old != nil {
		old.cancel()
	}
	ctx, cancel := context.WithCancel(context.Background())
	v := &session{tunnel: tunnel, token: credential, ctx: ctx, cancel: cancel}
	s.sessions[host] = v
	return func() {
		s.mu.Lock()
		defer s.mu.Unlock()
		if s.sessions[host] == v {
			delete(s.sessions, host)
		}
		cancel()
	}, nil
}
func (s *Server) Disconnect(host string) {
	s.mu.Lock()
	v := s.sessions[host]
	if v != nil {
		v.cancel()
		delete(s.sessions, host)
	}
	s.mu.Unlock()
	if v != nil {
		v.drained.Wait()
	}
}
func (s *Server) SetPolicy(host string, limits Limits, disabled, revoked bool) error {
	if err := s.Store.Policy(host, limits, disabled, revoked); err != nil {
		return err
	}
	s.Disconnect(host)
	return nil
}
func (s *Server) Close() {
	s.mu.Lock()
	defer s.mu.Unlock()
	for host, v := range s.sessions {
		v.cancel()
		delete(s.sessions, host)
	}
}
func (s *Server) Serve(ctx context.Context, l net.Listener) error {
	parent := ctx
	ctx, cancel := context.WithCancel(ctx)
	stop := context.AfterFunc(ctx, func() { l.Close() })
	defer stop()
	var workers sync.WaitGroup
	defer func() { cancel(); l.Close(); workers.Wait() }()
	pending := make(chan struct{}, 256)
	for {
		c, err := l.Accept()
		if err != nil {
			if parent.Err() != nil {
				return nil
			}
			return err
		}
		select {
		case pending <- struct{}{}:
		default:
			c.Close()
			continue
		}
		workers.Add(1)
		go func() { defer workers.Done(); defer func() { <-pending }(); s.forward(ctx, c) }()
	}
}
func (s *Server) forward(ctx context.Context, c net.Conn) {
	defer c.Close()
	stop := context.AfterFunc(ctx, func() { c.Close() })
	defer stop()
	c.SetReadDeadline(time.Now().Add(5 * time.Second))
	host, hello, err := clientHello(c)
	if err != nil {
		return
	}
	c.SetReadDeadline(time.Time{})
	if s.Control != nil && host == s.ControlHost {
		s.Control.forward(ctx, c, hello)
		return
	}
	host = s.Store.canonical(host)
	s.mu.Lock()
	v := s.sessions[host]
	d, ok := s.Store.Device(host)
	if !ok || v == nil || !s.Store.authenticated(host, v.token) || v.active >= d.Limits.Connections || d.Bytes >= d.Limits.MonthlyBytes && d.Month == time.Now().UTC().Format("2006-01") {
		s.mu.Unlock()
		return
	}
	v.active++
	v.drained.Add(1)
	s.mu.Unlock()
	defer func() { s.mu.Lock(); v.active--; s.mu.Unlock(); v.drained.Done() }()
	bounded, cancel := context.WithCancel(v.ctx)
	defer cancel()
	stop = context.AfterFunc(ctx, cancel)
	defer stop()
	timeout := time.AfterFunc(10*time.Second, cancel)
	upstream, err := v.tunnel.Open(bounded)
	timeout.Stop()
	if err != nil {
		return
	}
	defer upstream.Close()
	rawPublic, remote := c, upstream
	stop = context.AfterFunc(bounded, func() { rawPublic.Close(); remote.Close() })
	defer stop()
	idle := s.IdleTimeout
	if idle <= 0 {
		idle = 2 * time.Minute
	}
	watchdog := time.AfterFunc(idle, cancel)
	defer watchdog.Stop()
	upstream = idleStream{ReadWriteCloser: upstream, timer: watchdog, timeout: idle}
	public := idleStream{ReadWriteCloser: c, timer: watchdog, timeout: idle}
	if err = s.write(bounded, v, host, upstream, hello); err != nil {
		return
	}
	done := make(chan struct{})
	go func() { defer close(done); s.copy(bounded, v, host, upstream, public); cancel() }()
	s.copy(bounded, v, host, public, upstream)
	cancel()
	<-done
}
func (s *Server) copy(ctx context.Context, v *session, host string, dst io.Writer, src io.Reader) error {
	b := make([]byte, 4096)
	defer clear(b)
	for {
		n, err := src.Read(b)
		if n > 0 {
			if e := s.write(ctx, v, host, dst, b[:n]); e != nil {
				return e
			}
		}
		if err != nil {
			return err
		}
	}
}
func (s *Server) write(ctx context.Context, v *session, host string, dst io.Writer, b []byte) error {
	for len(b) > 0 {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		d, _ := s.Store.Device(host)
		size := len(b)
		if size > 4096 {
			size = 4096
		}
		if int64(size) > d.Limits.BytesPerSecond {
			size = int(d.Limits.BytesPerSecond)
		}
		n, err := s.Store.charge(host, size)
		if err != nil {
			return err
		}
		s.mu.Lock()
		now := time.Now()
		if v.next.Before(now) {
			v.next = now
		}
		v.next = v.next.Add(time.Duration(n) * time.Second / time.Duration(d.Limits.BytesPerSecond))
		wait := time.Until(v.next)
		s.mu.Unlock()
		timer := time.NewTimer(wait)
		select {
		case <-ctx.Done():
			timer.Stop()
			return ctx.Err()
		case <-timer.C:
		}
		written, err := dst.Write(b[:n])
		if err != nil {
			return err
		}
		if written != n {
			return io.ErrShortWrite
		}
		b = b[n:]
	}
	return nil
}

type idleStream struct {
	io.ReadWriteCloser
	timer   *time.Timer
	timeout time.Duration
}

func (c idleStream) Read(b []byte) (int, error) {
	n, err := c.ReadWriteCloser.Read(b)
	if n > 0 {
		c.timer.Reset(c.timeout)
	}
	return n, err
}
func (c idleStream) Write(b []byte) (int, error) {
	n, err := c.ReadWriteCloser.Write(b)
	if n > 0 {
		c.timer.Reset(c.timeout)
	}
	return n, err
}
