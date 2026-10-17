package applepeer

import (
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"net"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Config struct {
	BlobCapture func(context.Context, contracts.Capture, *Wire)

	Commands      *Commands
	Socket        string
	Secret        []byte
	Checkpoints   func(context.Context) (map[contracts.Source]contracts.Checkpoint, error)
	NextFence     func(context.Context) (uint64, error)
	CommitCapture func(context.Context, contracts.Capture) (bool, error)
	SourceState   func(context.Context, contracts.Source, string, contracts.Coverage, []string, string) error
}

type Wire struct {
	Conn        *net.UnixConn
	write       sync.Mutex
	blobMu      sync.Mutex
	blobPending map[string]chan contracts.IPCFrame
}

func (w *Wire) Send(frame contracts.IPCFrame) (int, error) {
	w.write.Lock()
	defer w.write.Unlock()
	w.Conn.SetWriteDeadline(time.Now().Add(10 * time.Second))
	return contracts.WriteFrame(w.Conn, frame)
}
func (w *Wire) Receive() (contracts.IPCFrame, int, error) {
	var f contracts.IPCFrame
	w.Conn.SetReadDeadline(time.Now().Add(15 * time.Second))
	n, err := contracts.ReadFrame(w.Conn, &f)
	return f, n, err
}

func nonce() (string, error) {
	var b [32]byte
	_, err := rand.Read(b[:])
	return hex.EncodeToString(b[:]), err
}
func proof(secret []byte, role, a, b string) string {
	m := hmac.New(sha256.New, secret)
	m.Write([]byte("apple-bridge-ipc/1\x00" + role + "\x00" + a + "\x00" + b))
	return hex.EncodeToString(m.Sum(nil))
}

func Authenticate(w *Wire, secret []byte, server bool) error {
	if err := checkUID(w.Conn); err != nil {
		return err
	}
	if len(secret) != 32 {
		return contracts.ErrUnauthorized
	}
	if server {
		a, err := nonce()
		if err != nil {
			return err
		}
		if _, err = w.Send(contracts.IPCFrame{T: "Challenge", Nonce: a}); err != nil {
			return err
		}
		f, _, err := w.Receive()
		if err != nil {
			return err
		}
		expected := proof(secret, "helper", a, f.Nonce)
		if f.T != "Authenticate" || len(f.Nonce) != 64 || !hmac.Equal([]byte(f.Proof), []byte(expected)) {
			return contracts.ErrUnauthorized
		}
		_, err = w.Send(contracts.IPCFrame{T: "Authenticated", Proof: proof(secret, "gateway", a, f.Nonce)})
		return err
	}
	f, _, err := w.Receive()
	if err != nil {
		return err
	}
	if f.T != "Challenge" || len(f.Nonce) != 64 {
		return contracts.ErrUnauthorized
	}
	b, err := nonce()
	if err != nil {
		return err
	}
	if _, err = w.Send(contracts.IPCFrame{T: "Authenticate", Nonce: b, Proof: proof(secret, "helper", f.Nonce, b)}); err != nil {
		return err
	}
	answer, _, err := w.Receive()
	if err != nil {
		return err
	}
	if answer.T != "Authenticated" || !hmac.Equal([]byte(answer.Proof), []byte(proof(secret, "gateway", f.Nonce, b))) {
		return contracts.ErrUnauthorized
	}
	return nil
}

func Serve(ctx context.Context, cfg Config) error {
	if err := os.MkdirAll(filepath.Dir(cfg.Socket), 0700); err != nil {
		return err
	}
	if err := os.Chmod(filepath.Dir(cfg.Socket), 0700); err != nil {
		return err
	}
	if info, err := os.Lstat(cfg.Socket); err == nil {
		if info.Mode()&os.ModeSocket == 0 {
			return contracts.ErrConflict
		}
		if err = os.Remove(cfg.Socket); err != nil {
			return err
		}
	}
	listener, err := net.ListenUnix("unix", &net.UnixAddr{Name: cfg.Socket, Net: "unix"})
	if err != nil {
		return err
	}
	defer listener.Close()
	defer os.Remove(cfg.Socket)
	if err = os.Chmod(cfg.Socket, 0600); err != nil {
		return err
	}
	var mu sync.Mutex
	var active *net.UnixConn
	go func() {
		<-ctx.Done()
		listener.Close()
		mu.Lock()
		if active != nil {
			active.Close()
		}
		mu.Unlock()
	}()
	for {
		conn, err := listener.AcceptUnix()
		if err != nil {
			if ctx.Err() != nil {
				return nil
			}
			return err
		}
		go func() {
			defer conn.Close()
			wire := &Wire{Conn: conn}
			if Authenticate(wire, cfg.Secret, true) != nil {
				return
			}
			mu.Lock()
			if active != nil {
				active.Close()
			}
			active = conn
			fence, err := cfg.NextFence(ctx)
			mu.Unlock()
			if err != nil {
				return
			}
			checkpoints, err := cfg.Checkpoints(ctx)
			if err != nil {
				return
			}
			if _, err = wire.Send(contracts.IPCFrame{T: "Open", Protocol: contracts.IPCProtocol, OwnerFence: fence, Checkpoints: checkpoints, ByteCredit: contracts.ByteCredit}); err != nil {
				return
			}
			ready, _, err := wire.Receive()
			if err != nil || ready.T != "Ready" || ready.Protocol != contracts.IPCProtocol {
				return
			}
			var commands *commandSession
			if cfg.Commands != nil {
				commands = cfg.Commands.attach(wire, fence)
				defer cfg.Commands.detach(commands)
			}
			defer func() {
				mu.Lock()
				defer mu.Unlock()
				if active == conn {
					active = nil
					for source, coverage := range ready.Coverage {
						cfg.SourceState(ctx, source, "degraded", coverage, ready.Capabilities[source], "helper_disconnected")
					}
				}
			}()
			for source, coverage := range ready.Coverage {
				if err = cfg.SourceState(ctx, source, "ready", coverage, ready.Capabilities[source], ""); err != nil {
					return
				}
			}
			session, stop := context.WithCancel(ctx)
			defer stop()
			go func() {
				tick := time.NewTicker(5 * time.Second)
				defer tick.Stop()
				for {
					select {
					case <-session.Done():
						return
					case <-tick.C:
						if _, err := wire.Send(contracts.IPCFrame{T: "Ping"}); err != nil {
							conn.Close()
							return
						}
					}
				}
			}()
			for {
				f, n, err := wire.Receive()
				if err != nil {
					break
				}
				switch f.T {
				case "Prepared", "Rejected", "Outcome":
					if cfg.Commands != nil {
						cfg.Commands.receive(commands, f)
					}
				case "Capture":
					if f.Capture == nil || n > contracts.ByteCredit {
						return
					}
					mu.Lock()
					if active != conn {
						mu.Unlock()
						return
					}
					degraded, err := cfg.CommitCapture(ctx, *f.Capture)
					mu.Unlock()
					if err != nil {
						if err == contracts.ErrReset {
							wire.Send(contracts.IPCFrame{T: "Reset", Source: f.Capture.Source})
							continue
						}
						cfg.SourceState(ctx, f.Capture.Source, "degraded", contracts.Coverage{Fields: "partial", Missing: []string{"capture_failed"}}, nil, "capture_failed")
						return
					}
					if cfg.BlobCapture != nil {
						cfg.BlobCapture(session, *f.Capture, wire)
					}
					if degraded {
						cfg.SourceState(ctx, f.Capture.Source, "degraded", contracts.Coverage{Fields: "partial", Missing: []string{"incomplete_scan_absence_ignored"}}, nil, "incomplete_scan")
					}
					if _, err = wire.Send(contracts.IPCFrame{T: "Committed", Source: f.Capture.Source, Checkpoint: f.Capture.Checkpoint, Credit: n}); err != nil {
						return
					}
				case "BlobChunk", "BlobUnavailable":
					wire.deliverBlob(f)
				case "SourceState":
					if err = cfg.SourceState(ctx, f.Source, f.State, f.Coverage[f.Source], f.Capabilities[f.Source], f.Reason); err != nil {
						return
					}
				case "Ping":
					if _, err = wire.Send(contracts.IPCFrame{T: "Pong"}); err != nil {
						return
					}
				case "Pong":
				case "Close":
					return
				default:
					return
				}
			}
		}()
	}
}
