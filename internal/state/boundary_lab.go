//go:build lab

package state

import (
	"os"
	"path/filepath"
	"time"
)

func initializeApplyBoundary(s *Store) {
	s.applyBoundary = func() {
		arm := filepath.Join(s.home, "lab-apply-armed")
		if os.Remove(arm) != nil {
			return
		}
		if os.WriteFile(filepath.Join(s.home, "lab-apply-reached"), []byte("write_started_before_commit"), 0600) != nil {
			panic("apply boundary signal failed")
		}
		for {
			time.Sleep(time.Hour)
		}
	}
}

func captureDiscarded(home string, pending bool) {
	if pending {
		os.WriteFile(filepath.Join(home, "lab-capture-discarded"), []byte("discarded"), 0600)
	}
}
