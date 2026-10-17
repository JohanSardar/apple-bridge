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
	done := make(chan struct{})
	go func() { p.copyUpstream(upstream, downstream); upstream.Close(); close(done) }()
	buffer := make([]byte, 32*1024)
	for {
		n, err := upstream.Read(buffer)
		if n > 0 {
			p.mu.Lock()
			delay, cut, partitioned, blackhole := p.delay, p.cut, p.partitioned, p.blackhole
			if cut > 0 {
				p.cut = max(0, cut-n)
			}
			p.mu.Unlock()
			if partitioned {
				break
			}
			if blackhole {
				continue
			}
			if delay > 0 {
				time.Sleep(delay)
			}
			if cut > 0 && n >= cut {
				downstream.Write(buffer[:cut])
				break
			}
			if _, err := downstream.Write(buffer[:n]); err != nil {
				break
			}
		}
		if err != nil {
			break
		}
	}
	downstream.Close()
	upstream.Close()
	<-done
}

func (p *FaultProxy) copyUpstream(dst, src net.Conn) {
	buffer := make([]byte, 32*1024)
	for {
		n, err := src.Read(buffer)
		if n > 0 {
			p.mu.Lock()
			drop := p.blackhole
			p.mu.Unlock()
			if !drop {
				if _, err := dst.Write(buffer[:n]); err != nil {
					return
				}
			}
		}
		if err != nil {
			return
		}
	}
}
