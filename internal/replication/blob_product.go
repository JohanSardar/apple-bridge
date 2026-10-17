//go:build !lab

package replication

import (
	"context"
	"github.com/coder/websocket"
)

func mutateBlobChunk(b []byte) {}

func writeSocketFrame(ctx context.Context, conn *websocket.Conn, kind websocket.MessageType, b []byte) error {
	return conn.Write(ctx, kind, b)
}
