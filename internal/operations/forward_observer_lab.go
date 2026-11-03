//go:build lab

package operations

import (
	"encoding/base64"
	"encoding/json"
	"os"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

var forwardObserverMu sync.Mutex

const forwardObserverTime = "2006-01-02T15:04:05.000000000Z"

func observeForward(header string, raw json.RawMessage) func(contracts.CommandResponse, error) {
	path := os.Getenv("APPLE_BRIDGE_LAB_FEED_LOG")
	if path == "" {
		return nil
	}
	start := time.Now().UTC()
	body := append(json.RawMessage(nil), raw...)
	return func(response contracts.CommandResponse, err error) {
		if err != nil {
			return
		}
		var request struct {
			Method string `json:"method"`
		}
		if json.Unmarshal(body, &request) != nil || request.Method != "submit" && request.Method != "status" && request.Method != "list" {
			return
		}
		outcome := map[string]any{"status": 200}
		if response.Error != nil {
			if response.Error.Code == "unauthorized" {
				return
			}
			outcome = map[string]any{"status": 400, "error": response.Error.Code}
		} else if request.Method == "list" {
			var rows []json.RawMessage
			if json.Unmarshal(response.Result, &rows) != nil {
				return
			}
			outcome["count"] = len(rows)
		} else {
			var record struct {
				State       string `json:"state"`
				Disposition string `json:"disposition"`
			}
			if json.Unmarshal(response.Result, &record) != nil {
				return
			}
			outcome["state"], outcome["disposition"] = record.State, record.Disposition
		}
		parts := strings.Split(header, ".")
		decoded, decodeErr := base64.RawURLEncoding.DecodeString(parts[0])
		var delegation map[string]json.RawMessage
		if decodeErr != nil || json.Unmarshal(decoded, &delegation) != nil {
			return
		}
		kept := map[string]json.RawMessage{}
		for _, key := range []string{"local_level", "local_principal", "local_grants", "command_id", "request_hash"} {
			if value, ok := delegation[key]; ok {
				kept[key] = value
			}
		}
		line, marshalErr := json.Marshal(map[string]any{"method": request.Method, "start": start.Format(forwardObserverTime), "end": time.Now().UTC().Format(forwardObserverTime), "body": base64.StdEncoding.EncodeToString(body), "delegation": kept, "outcome": outcome})
		if marshalErr != nil {
			return
		}
		forwardObserverMu.Lock()
		defer forwardObserverMu.Unlock()
		file, openErr := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
		if openErr != nil {
			return
		}
		defer file.Close()
		file.Write(append(line, '\n'))
	}
}
