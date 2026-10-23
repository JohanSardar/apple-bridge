package state

import (
	"bytes"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func noteContentVersion(old contracts.Record, data json.RawMessage) (json.RawMessage, error) {
	var next, previous map[string]json.RawMessage
	if err := json.Unmarshal(data, &next); err != nil {
		return nil, err
	}
	if next == nil {
		return nil, contracts.ErrCorrupt
	}
	delete(next, "content_version")
	version := old.Version + 1
	if !old.Deleted && json.Unmarshal(old.Data, &previous) == nil {
		var fingerprint string
		json.Unmarshal(next["native_fingerprint"], &fingerprint)
		savedData := previous["content_version"]
		delete(previous, "content_version")
		before, _ := json.Marshal(previous)
		after, _ := json.Marshal(next)
		identical := bytes.Equal(before, after)
		same := fingerprint != ""
		for _, field := range []string{"native_fingerprint", "title", "plain_text", "locked", "folder_id", "recently_deleted", "read_only"} {
			same = same && bytes.Equal(next[field], previous[field])
		}
		if same || identical {
			version = old.Version
			var saved uint64
			if json.Unmarshal(savedData, &saved) == nil && saved > 0 && saved <= old.Version {
				version = saved
			}
		}
	}
	next["content_version"], _ = json.Marshal(version)
	return json.Marshal(next)
}
