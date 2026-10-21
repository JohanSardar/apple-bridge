package world

import (
	"context"
	"crypto/tls"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"io"
	"net"
	"net/http"
	"path/filepath"
	"slices"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

type frameFaults struct {
	atomicOffers            int
	legacyOffers            int
	atomicWelcomes          int
	captureReset            bool
	forceSnapshot           bool
	partialFields           int
	partialFrames           int
	partialAckWithoutCursor int
	partialAckWithCursor    int
	partialBytes            int
	resetFrames             int
	legacyWelcomes          int
	helloSequences          []uint64
	server                  tls.Certificate
	client                  tls.Certificate
	dropBatch               bool
	dropAcks                bool
	dropped                 int
	connections             int
	creditHighWater         int
}

func (w *World) configureFrameFaults() error {
	gateway, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return err
	}
	endpoint := w.Replicas[0]
	var certificate tls.Certificate
	if endpoint.Remote {
		public, err := w.execute(true, "cat", filepath.Join(endpoint.Home, "identity.pem"))
		if err != nil {
			return err
		}
		key, err := w.execute(true, "cat", filepath.Join(endpoint.Home, "secrets", trust.Fingerprint([]byte("identity-key"))))
		if err != nil {
			return err
		}
		certificate, err = tls.X509KeyPair([]byte(public), []byte(key))
		if err != nil {
			return err
		}
	} else {
		replica, err := trust.Open(endpoint.Home)
		if err != nil {
			return err
		}
		certificate = replica.Certificate
	}
	w.proxy.mu.Lock()
	defer w.proxy.mu.Unlock()
	w.proxy.frames = &frameFaults{server: gateway.Certificate, client: certificate}
	return nil
}
func (w *World) DropBatch() error {
	if err := w.configureFrameFaults(); err != nil {
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
		if err := w.configureFrameFaults(); err != nil {
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
func (w *World) FeedCreditHighWater() int {
	w.proxy.mu.Lock()
	defer w.proxy.mu.Unlock()
	return w.proxy.frames.creditHighWater
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
		outstanding := 0
		partialReceipts := []bool{}
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
				if frame.T == "Hello" {
					if slices.Contains(frame.Features, "capture_atomic") {
						p.frames.atomicOffers++
					} else {
						p.frames.legacyOffers++
					}
					if frame.Cursor != nil {
						p.frames.helloSequences = append(p.frames.helloSequences, frame.Cursor.Sequence)
					}
					if p.frames.forceSnapshot {
						frame.Cursor = nil
						p.frames.forceSnapshot = false
						data, _ = json.Marshal(frame)
					}
				}
				drop := p.frames.dropAcks && frame.T == "Ack"
				if frame.T == "Ack" && !drop {
					if len(partialReceipts) > 0 {
						if partialReceipts[0] {
							if frame.Cursor == nil {
								p.frames.partialAckWithoutCursor++
							} else {
								p.frames.partialAckWithCursor++
							}
						}
						partialReceipts = partialReceipts[1:]
					}
					outstanding -= frame.Credits
				}
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
			switch frame.T {
			case "Welcome", "Reset", "SnapshotBegin", "SnapshotPage", "SnapshotEnd", "Batch":
				outstanding += len(data)
				partialReceipts = append(partialReceipts, frame.T == "Batch" && frame.Batch != nil && frame.Batch.Partial)
				p.frames.creditHighWater = max(p.frames.creditHighWater, outstanding)
			}
			if frame.T == "Batch" {
				var object map[string]json.RawMessage
				if json.Unmarshal(data, &object) == nil {
					if _, exists := object["partial"]; exists {
						p.frames.partialFields++
					}
				}
			}
			resetCapture := false
			if frame.T == "Welcome" && slices.Contains(frame.Features, "capture_atomic") {
				p.frames.atomicWelcomes++
			}
			if frame.T == "Welcome" && len(frame.Features) == 0 {
				p.frames.legacyWelcomes++
			}
			if frame.T == "Batch" && frame.Batch != nil && frame.Batch.Partial {
				p.frames.partialFrames++
				p.frames.partialBytes += len(data)
				if p.frames.captureReset {
					p.frames.captureReset = false
					p.frames.forceSnapshot = true
					p.frames.resetFrames++
					resetCapture = true
				}
			}
			drop := p.frames.dropBatch && frame.T == "Batch"
			if drop {
				p.frames.dropBatch = false
				p.frames.dropped++
				outstanding -= len(data)
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
			if resetCapture {
				reset, _ := json.Marshal(contracts.FeedFrame{T: "Reset", Reason: "capture_fixture"})
				downstream.Write(ctx, websocket.MessageText, reset)
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
