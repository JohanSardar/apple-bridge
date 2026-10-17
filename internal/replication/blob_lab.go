//go:build lab

package replication

import (
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
