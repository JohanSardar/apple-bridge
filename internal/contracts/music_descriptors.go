package contracts

import (
	"encoding/json"
	"strings"
)

func init() {
	RegisterSource("music", "Music", true)
	registerDescriptors(func() []OperationDescriptor {
		out := []OperationDescriptor{
			{Name: "music.search_catalog", Version: 1, Kind: QueryOp, Source: "music", Scope: "music:read", Input: Query{}, Output: QueryResult{}, Description: "Search the public iTunes catalog; bounded transient metadata"},
			{Name: "music.now_playing", Version: 1, Kind: QueryOp, Source: "music", Scope: "music:read", Input: Query{}, Output: QueryResult{}, Description: "Read the authorized Music player state"},
		}
		for _, op := range []string{"play", "pause", "next", "previous", "set_volume", "play_item"} {
			out = append(out, OperationDescriptor{Name: "music." + op, Version: 1, Kind: CommandOp, Source: "music", Scope: "music:write", Capability: "music." + op, Input: Command{}, Output: CommandRecord{}, Description: "Control Music with player state readback"})
		}
		return out
	}, 90, 91, 92, 93, 94, 95, 96, 97)
}

type MusicCommandArgs struct {
	Volume *int   `json:"volume,omitempty"`
	ID     string `json:"id,omitempty"`
}

func ValidateMusicCommand(c Command) error {
	var a MusicCommandArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ErrInvalid
	}
	var fields map[string]json.RawMessage
	json.Unmarshal(c.Args, &fields)
	switch c.Op {
	case "music.play", "music.pause", "music.next", "music.previous":
		if len(fields) != 0 {
			return ErrInvalid
		}
	case "music.set_volume":
		if len(fields) != 1 || a.Volume == nil || *a.Volume < 0 || *a.Volume > 100 {
			return ErrInvalid
		}
	case "music.play_item":
		if len(fields) != 1 || len(a.ID) != 16 || strings.Trim(a.ID, "0123456789ABCDEFabcdef") != "" {
			return ErrInvalid
		}
	default:
		return ErrInvalid
	}
	return nil
}
func MusicCompletionEvidence(raw json.RawMessage, hash string) bool {
	var e struct {
		Fingerprint string `json:"fingerprint"`
		Verified    bool   `json:"verified"`
		Player      struct {
			State  string `json:"state"`
			Volume *int   `json:"volume"`
		} `json:"player"`
	}
	return json.Unmarshal(raw, &e) == nil && hash != "" && e.Fingerprint == hash && e.Verified && e.Player.Volume != nil && *e.Player.Volume >= 0 && *e.Player.Volume <= 100 && (e.Player.State == "playing" || e.Player.State == "paused" || e.Player.State == "stopped")
}
