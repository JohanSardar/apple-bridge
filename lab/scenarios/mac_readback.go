package scenarios

import "encoding/json"

func macCompletionReadback(raw json.RawMessage, hash string) bool {
	var evidence struct {
		Observed    bool            `json:"observed"`
		Fingerprint string          `json:"fingerprint"`
		Readback    json.RawMessage `json:"readback"`
	}
	if json.Unmarshal(raw, &evidence) != nil || !evidence.Observed || hash == "" || evidence.Fingerprint != hash {
		return false
	}
	return len(evidence.Readback) > 0 && string(evidence.Readback) != "null"
}
