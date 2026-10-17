//go:build lab

package runtime

import (
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
