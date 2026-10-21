//go:build lab

package runtime

import (
	"io"
	"os"
	"path/filepath"
	"time"
)

func captureBoundary(home string) {
	if os.Remove(filepath.Join(home, "lab-capture-armed")) != nil {
		return
	}
	if os.WriteFile(filepath.Join(home, "lab-capture-reached"), []byte("capture_committed_before_acknowledgment"), 0600) != nil {
		panic("capture boundary signal failed")
	}
	for {
		time.Sleep(time.Hour)
	}
}

func commandCaptureBoundary(home string) {
	if os.Remove(filepath.Join(home, "lab-command-capture-armed")) != nil {
		return
	}
	if os.WriteFile(filepath.Join(home, "lab-command-capture-reached"), []byte("committed_before_receipt"), 0600) != nil {
		panic("command capture boundary signal failed")
	}
	gate, err := os.Open(filepath.Join(home, "lab-command-capture-release"))
	if err != nil {
		panic("command capture boundary open failed")
	}
	defer gate.Close()
	var release [1]byte
	if _, err = io.ReadFull(gate, release[:]); err != nil {
		panic("command capture boundary release failed")
	}
}
