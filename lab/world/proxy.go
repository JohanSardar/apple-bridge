package world

import (
	"context"
	"net"
	"sync"
	"time"
)

type FaultProxy struct {
	frames      *frameFaults
	listener    net.Listener
	target      string
	mu          sync.Mutex
	blackhole   bool
	partitioned bool
	delay       time.Duration
	delayBoth   bool
	cut         int
	connections map[net.Conn]struct{}
}

func NewFaultProxy(address, target string) (*FaultProxy, error) {
	listener, err := net.Listen("tcp", address)
	if err != nil {
		return nil, err
	}
	p := &FaultProxy{listener: listener, target: target, connections: map[net.Conn]struct{}{}}
	go p.accept()
	return p, nil
}
func (p *FaultProxy) Address() string { return p.listener.Addr().String() }
func (p *FaultProxy) Partition() {
	p.mu.Lock()
	defer p.mu.Unlock()
	p.partitioned = true
	for conn := range p.connections {
		conn.Close()
	}
}
func (p *FaultProxy) Heal() {
	p.mu.Lock()
	p.partitioned = false
	p.blackhole = false
	p.mu.Unlock()
}
func (p *FaultProxy) Blackhole()                { p.mu.Lock(); p.blackhole = true; p.mu.Unlock() }
func (w *World) Blackhole()                     { w.proxy.Blackhole() }
func (p *FaultProxy) Delay(delay time.Duration) { p.mu.Lock(); p.delay = delay; p.mu.Unlock() }
func (p *FaultProxy) CutAfter(bytes int)        { p.mu.Lock(); p.cut = bytes; p.mu.Unlock() }
func (p *FaultProxy) Close() {
	p.listener.Close()
	p.Partition()
}
func (p *FaultProxy) accept() {
	for {
		conn, err := p.listener.Accept()
		if err != nil {
			return
		}
		go p.forward(conn)
	}
}
func (p *FaultProxy) forward(downstream net.Conn) {
	defer downstream.Close()
	p.mu.Lock()
	if p.partitioned {
		p.mu.Unlock()
		return
	}
	p.connections[downstream] = struct{}{}
	var frames frameFaults
	framed := p.frames != nil
	if framed {
		frames = *p.frames
	}
	p.mu.Unlock()
	defer func() { p.mu.Lock(); delete(p.connections, downstream); p.mu.Unlock() }()
	if framed {
		p.relayFrames(downstream, frames)
		return
	}
	upstream, err := (&net.Dialer{Timeout: 5 * time.Second}).DialContext(context.Background(), "tcp", p.target)
	if err != nil {
		return
	}
	defer upstream.Close()
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	done := make(chan struct{}, 2)
	go func() { p.copyDelayed(ctx, upstream, downstream, true); done <- struct{}{} }()
	go func() { p.copyDelayed(ctx, downstream, upstream, false); done <- struct{}{} }()
	<-done
	cancel()
	downstream.Close()
	upstream.Close()
	<-done
}

type delayedChunk struct {
	bytes []byte
	at    time.Time
	cut   bool
}

func (p *FaultProxy) copyDelayed(ctx context.Context, dst, src net.Conn, upstream bool) {
	chunks := make(chan delayedChunk, 64)
	go func() {
		defer close(chunks)
		for {
			bytes := make([]byte, 32*1024)
			n, err := src.Read(bytes)
			if n > 0 {
				received := time.Now()
				p.mu.Lock()
				delay := p.delay
				if upstream && !p.delayBoth {
					delay = 0
				}
				drop, partitioned := p.blackhole, p.partitioned
				cut := false
				if !upstream && p.cut > 0 {
					if n >= p.cut {
						n = p.cut
						cut = true
					}
					p.cut = max(0, p.cut-n)
				}
				p.mu.Unlock()
				if partitioned {
					return
				}
				if !drop {
					select {
					case chunks <- delayedChunk{bytes: bytes[:n], at: received.Add(delay), cut: cut}:
					case <-ctx.Done():
						return
					}
				}
				if cut {
					return
				}
			}
			if err != nil {
				return
			}
		}
	}()
	for {
		select {
		case <-ctx.Done():
			return
		case chunk, ok := <-chunks:
			if !ok {
				return
			}
			timer := time.NewTimer(max(0, time.Until(chunk.at)))
			select {
			case <-ctx.Done():
				timer.Stop()
				return
			case <-timer.C:
			}
			p.mu.Lock()
			drop := p.blackhole || p.partitioned
			p.mu.Unlock()
			if !drop {
				if _, err := dst.Write(chunk.bytes); err != nil {
					return
				}
			}
			if chunk.cut {
				return
			}
		}
	}
}

func (w *World) DelayBothDirections(delay time.Duration) {
	w.proxy.mu.Lock()
	w.proxy.delay = delay
	w.proxy.delayBoth = delay > 0
	w.proxy.mu.Unlock()
}
