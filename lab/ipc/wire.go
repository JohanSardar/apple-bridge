package ipc

import (
	"context"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net"
	"sync"
	"time"
)

type Wire struct {
	Conn *net.UnixConn
	mu   sync.Mutex
}

func (w *Wire) Send(f contracts.IPCFrame) (int, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	return w.SendContext(ctx, f)
}
func (w *Wire) SendContext(ctx context.Context, f contracts.IPCFrame) (int, error) {
	w.mu.Lock()
	defer w.mu.Unlock()
	if err := ctx.Err(); err != nil {
		return 0, err
	}
	deadline, ok := ctx.Deadline()
	if !ok {
		deadline = time.Now().Add(10 * time.Second)
	}
	w.Conn.SetWriteDeadline(deadline)
	return contracts.WriteFrame(w.Conn, f)
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

func Authenticate(w *Wire, secret []byte) error {
	if len(secret) != 32 {
		return contracts.ErrUnauthorized
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
