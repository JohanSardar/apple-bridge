package scenarios

import (
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadStatusCapture(w *world.World, id string, add func(string, bool)) {
	target, err := captureObserverChanges(w, contracts.Messages, "messages.message", "status-capture", 150)
	add("oversized_capture_committed", err == nil)
	if err != nil {
		return
	}
	err = w.Await(10*time.Second, func() bool {
		current, err := w.Cursor(w.Replicas[0])
		return err == nil && current.Epoch == target.Epoch && current.Sequence >= target.Sequence
	})
	add("oversized_capture_applied_atomically", err == nil)
	err = w.Await(5*time.Second, func() bool {
		raw, _ := os.ReadFile(filepath.Join(w.Replicas[0].Home, "command-observe-reset"))
		return string(raw) == id
	})
	add("command_status_iterator_received_actual_oversized_reset", err == nil)
}
