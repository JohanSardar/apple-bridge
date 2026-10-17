package world

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"io"
	"net"
	"net/http"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

type frameFaults struct {
	server      tls.Certificate
	client      tls.Certificate
	dropBatch   bool
	dropAcks    bool
	dropped     int
	connections int
}

func (p *FaultProxy) Frames(gatewayHome, replicaHome string) error {
	gateway, err := trust.Open(gatewayHome)
	if err != nil {
		return err
	}
	replica, err := trust.Open(replicaHome)
	if err != nil {
		return err
	}
	p.mu.Lock()
	defer p.mu.Unlock()
	p.frames = &frameFaults{server: gateway.Certificate, client: replica.Certificate}
	return nil
}
func (w *World) DropBatch() error {
	if err := w.proxy.Frames(w.Gateway.Home, w.Replicas[0].Home); err != nil {
		return err
	}
	w.proxy.Partition()
	w.proxy.Heal()
	w.proxy.mu.Lock()
	w.proxy.frames.dropBatch = true
	w.proxy.mu.Unlock()
	return nil
}
func (w *World) SuppressAcks(enabled bool) error {
	w.proxy.mu.Lock()
	configured := w.proxy.frames != nil
	w.proxy.mu.Unlock()
	if !configured {
		if err := w.proxy.Frames(w.Gateway.Home, w.Replicas[0].Home); err != nil {
			return err
		}
		w.proxy.Partition()
		w.proxy.Heal()
	}
	w.proxy.mu.Lock()
	w.proxy.frames.dropAcks = enabled
	w.proxy.mu.Unlock()
	return nil
}
func (w *World) DroppedFrames() int {
	w.proxy.mu.Lock()
	defer w.proxy.mu.Unlock()
	return w.proxy.frames.dropped
}
func (w *World) FeedConnections() int {
	w.proxy.mu.Lock()
	defer w.proxy.mu.Unlock()
	return w.proxy.frames.connections
}

type singleListener struct {
	conn net.Conn
	once sync.Once
}

func (l *singleListener) Accept() (net.Conn, error) {
	var conn net.Conn
	l.once.Do(func() { conn = l.conn })
	if conn == nil {
		return nil, io.EOF
	}
	return conn, nil
}
func (l *singleListener) Close() error   { return nil }
func (l *singleListener) Addr() net.Addr { return l.conn.LocalAddr() }

func (p *FaultProxy) relayFrames(conn net.Conn, config frameFaults) {
	secure := tls.Server(conn, &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{config.server}, ClientAuth: tls.RequireAnyClientCert})
	done := make(chan struct{})
	server := &http.Server{ReadHeaderTimeout: 10 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer close(done)
		downstream, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer downstream.CloseNow()
		downstream.SetReadLimit(contracts.MaxFrame)
		transport := &http.Transport{TLSClientConfig: &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{config.client}, InsecureSkipVerify: true}}
		defer transport.CloseIdleConnections()
		ctx, cancel := context.WithCancel(r.Context())
		defer cancel()
		upstream, _, err := websocket.Dial(ctx, "wss://"+p.target+"/v1/feed", &websocket.DialOptions{HTTPClient: &http.Client{Transport: transport}})
		if err != nil {
			return
		}
		defer upstream.CloseNow()
		upstream.SetReadLimit(contracts.MaxFrame)
		p.mu.Lock()
		p.frames.connections++
		p.mu.Unlock()
		go func() {
			defer cancel()
			for {
				kind, data, err := downstream.Read(ctx)
				if err != nil {
					return
				}
				var frame contracts.FeedFrame
				json.Unmarshal(data, &frame)
				p.mu.Lock()
				drop := p.frames.dropAcks && frame.T == "Ack"
				p.mu.Unlock()
				if drop {
					continue
				}
				if upstream.Write(ctx, kind, data) != nil {
					return
				}
			}
		}()
		for {
			kind, data, err := upstream.Read(ctx)
			if err != nil {
				return
			}
			var frame contracts.FeedFrame
			json.Unmarshal(data, &frame)
			p.mu.Lock()
			drop := p.frames.dropBatch && frame.T == "Batch"
			if drop {
				p.frames.dropBatch = false
				p.frames.dropped++
			}
			p.mu.Unlock()
			if drop {
				cursor := contracts.Cursor{Epoch: frame.Batch.Epoch, SubscriptionRevision: frame.Batch.SubscriptionRevision, Sequence: frame.Batch.Through}
				ack, _ := json.Marshal(contracts.FeedFrame{T: "Ack", Credits: len(data), Cursor: &cursor})
				if upstream.Write(ctx, websocket.MessageText, ack) != nil {
					return
				}
				continue
			}
			if downstream.Write(ctx, kind, data) != nil {
				return
			}
		}
	})}
	server.Serve(&singleListener{conn: secure})
	select {
	case <-done:
	case <-time.After(2 * time.Minute):
	}
	server.Close()
}
