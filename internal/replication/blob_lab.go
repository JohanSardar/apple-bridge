//go:build lab

package replication

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
	"os"
	"path/filepath"
	"time"
)

func mutateBlobChunk(b []byte) {
	if path := os.Getenv("APPLE_BRIDGE_LAB_BLOB_CORRUPT"); path != "" && len(b) > 64 {
		if _, err := os.Stat(filepath.Dir(path)); err == nil {
			f, err := os.OpenFile(path, os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
			if err == nil {
				f.Close()
				b[64] ^= 0xff
			}
		}
	}
	if os.Getenv("APPLE_BRIDGE_LAB_BLOB_SLOW") != "" {
		time.Sleep(150 * time.Millisecond)
	}
}

func writeSocketFrame(ctx context.Context, conn *websocket.Conn, kind websocket.MessageType, b []byte) error {
	path := os.Getenv("APPLE_BRIDGE_LAB_BLOB_CUT_WINDOW")
	if path != "" && kind == websocket.MessageBinary {
		_, offset, _, err := contracts.DecodeBlobChunk(b)
		if err == nil && offset == contracts.BlobChunkSize {
			marker, err := os.OpenFile(path+".once", os.O_CREATE|os.O_EXCL|os.O_WRONLY, 0600)
			if err == nil {
				marker.Close()
				writer, err := conn.Writer(ctx, kind)
				if err != nil {
					return err
				}
				if _, err = writer.Write(b[:len(b)/2]); err != nil {
					writer.Close()
					return err
				}
				if err = os.WriteFile(path, []byte("in_flight"), 0600); err != nil {
					writer.Close()
					return err
				}
				timer := time.NewTimer(2 * time.Second)
				defer timer.Stop()
				select {
				case <-ctx.Done():
					writer.Close()
					return ctx.Err()
				case <-timer.C:
				}
				if _, err = writer.Write(b[len(b)/2:]); err != nil {
					writer.Close()
					return err
				}
				return writer.Close()
			}
		}
	}
	return conn.Write(ctx, kind, b)
}
