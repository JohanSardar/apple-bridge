package relay

import (
	"bytes"
	"context"
	"io"
	"net"
	"sync"
)

type ControlListener struct {
	ctx      context.Context
	cancel   context.CancelFunc
	incoming chan net.Conn
}

func NewControlListener(ctx context.Context) *ControlListener {
	ctx, cancel := context.WithCancel(ctx)
	return &ControlListener{ctx: ctx, cancel: cancel, incoming: make(chan net.Conn)}
}
func (l *ControlListener) Accept() (net.Conn, error) {
	select {
	case c := <-l.incoming:
		return c, nil
	case <-l.ctx.Done():
		return nil, net.ErrClosed
	}
}
func (l *ControlListener) Close() error   { l.cancel(); return nil }
func (l *ControlListener) Addr() net.Addr { return tunnelAddr("relay-control") }
func (l *ControlListener) forward(ctx context.Context, c net.Conn, hello []byte) {
	replay := &controlConn{Conn: c, reader: io.MultiReader(bytes.NewReader(hello), c), done: make(chan struct{})}
	defer replay.Close()
	select {
	case l.incoming <- replay:
	case <-l.ctx.Done():
		return
	case <-ctx.Done():
		return
	}
	select {
	case <-replay.done:
	case <-l.ctx.Done():
	case <-ctx.Done():
	}
}

type controlConn struct {
	net.Conn
	reader io.Reader
	once   sync.Once
	done   chan struct{}
}

func (c *controlConn) Read(b []byte) (int, error) { return c.reader.Read(b) }
func (c *controlConn) Close() error {
	err := c.Conn.Close()
	c.once.Do(func() { close(c.done) })
	return err
}
