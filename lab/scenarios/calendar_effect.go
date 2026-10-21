package scenarios

import (
	"bytes"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func calendarCommandEffect(w *world.World, command contracts.CommandRecord, operation string) int64 {
	var evidence struct {
		NativeKey string `json:"native_key"`
	}
	if json.Unmarshal(command.Evidence, &evidence) != nil || evidence.NativeKey == "" {
		return 0
	}
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(evidence.NativeKey)))
	trace, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"))
	if err != nil {
		return 0
	}
	var result int64
	for _, line := range bytes.Split(trace, []byte("\n")) {
		var row struct {
			Stage     string   `json:"stage"`
			Operation string   `json:"operation"`
			Keys      []string `json:"keys"`
			UptimeNS  int64    `json:"uptime_ns"`
		}
		if json.Unmarshal(line, &row) != nil || row.Stage != "calendar_native_effect" || row.Operation != operation {
			continue
		}
		for _, candidate := range row.Keys {
			if candidate == key {
				result = row.UptimeNS
			}
		}
	}
	return result
}
