//go:build lab

package runtime

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"path/filepath"
	"time"
)

func writeWebMetric(home string, event any) {
	if _, err := os.Stat(filepath.Join(home, "lab-web-metrics")); err != nil {
		return
	}
	raw, err := json.Marshal(event)
	if err != nil {
		return
	}
	file, err := os.OpenFile(filepath.Join(home, "web-metrics.jsonl"), os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err == nil {
		file.Write(append(raw, '\n'))
		file.Close()
	}
}
func webCaptureCommitted(home string, after, through uint64, at time.Time) {
	if at.IsZero() || through <= after {
		return
	}
	writeWebMetric(home, map[string]any{"stage": "commit", "after": after, "through": through, "at": at.UnixNano()})
}
func webFrameWritten(home string, cursor contracts.Cursor, at time.Time) {
	writeWebMetric(home, map[string]any{"stage": "written", "through": cursor.Sequence, "at": at.UnixNano()})
}
