//go:build lab

package contracts

import (
	"encoding/json"
	"os"
	"time"
)

type replayDecisions struct {
	Now     string            `json:"now"`
	Aliases map[string]string `json:"aliases"`
}

func loadReplayDecisions() *replayDecisions {
	path := os.Getenv("APPLE_BRIDGE_LAB_REPLAY")
	if path == "" {
		return nil
	}
	raw, err := os.ReadFile(path)
	if err != nil {
		panic("lab replay decisions unavailable")
	}
	var decisions replayDecisions
	if json.Unmarshal(raw, &decisions) != nil {
		panic("invalid lab replay decisions")
	}
	if _, err := time.Parse(time.RFC3339Nano, decisions.Now); err != nil {
		panic("invalid lab replay clock")
	}
	return &decisions
}

func DecisionNow() time.Time {
	if decisions := loadReplayDecisions(); decisions != nil {
		now, _ := time.Parse(time.RFC3339Nano, decisions.Now)
		return now.UTC()
	}
	return time.Now()
}

func ReplayRecordID(source Source, generation, collection, native string) (string, bool) {
	if decisions := loadReplayDecisions(); decisions != nil {
		key := string(source) + "\x00" + generation + "\x00" + collection + "\x00" + native
		id := decisions.Aliases[key]
		if id == "" {
			panic("lab replay alias missing")
		}
		return id, true
	}
	return "", false
}
