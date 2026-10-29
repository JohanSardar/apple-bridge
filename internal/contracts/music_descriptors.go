package contracts

import (
	"encoding/json"
	"strings"
)

func init() {
	RegisterSource("music", "Music", true)
	Collections["music.playlist"] = "mpl"
	Collections["music.library_item"] = "mli"
	messagesShape["music.playlist"] = map[string]fieldShape{"name": requiredField(shapeString), "items": requiredField(shapeArray), "count": requiredField(shapeNumber)}
	messagesShape["music.library_item"] = map[string]fieldShape{"name": requiredField(shapeString), "artist": requiredField(shapeString), "album": requiredField(shapeString), "favorite": requiredField(shapeBool)}
	registerDescriptors(func() []OperationDescriptor {
		out := []OperationDescriptor{
			{Name: "music.search_catalog", Version: 1, Kind: QueryOp, Source: "music", Scope: "music:read", Input: Query{}, Output: QueryResult{}, Description: "Search the public iTunes catalog; bounded transient metadata"},
			{Name: "music.now_playing", Version: 1, Kind: QueryOp, Source: "music", Scope: "music:read", Input: Query{}, Output: QueryResult{}, Description: "Read the authorized Music player state"},
		}
		for _, op := range []string{"play", "pause", "next", "previous", "set_volume", "play_item", "add_to_playlist", "favorite"} {
			out = append(out, OperationDescriptor{Name: "music." + op, Version: 1, Kind: CommandOp, Source: "music", Scope: "music:write", Capability: "music." + op, Input: Command{}, Output: CommandRecord{}, Description: "Control Music with player state readback"})
		}
		return out
	}, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99)
}

type MusicCommandArgs struct {
	PlaylistID string `json:"playlist_id,omitempty"`
	Favorite   *bool  `json:"favorite,omitempty"`
	Volume     *int   `json:"volume,omitempty"`
	ID         string `json:"id,omitempty"`
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
	case "music.favorite":
		if len(fields) != 2 || a.Favorite == nil || !musicRecordID(a.ID, "mli") {
			return ErrInvalid
		}
	case "music.add_to_playlist":
		if len(fields) != 2 || !musicRecordID(a.PlaylistID, "mpl") || !musicRecordID(a.ID, "mli") {
			return ErrInvalid
		}
	case "music.play_item":
		if len(fields) != 1 || !musicRecordID(a.ID, "mli") {
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

func musicRecordID(id, prefix string) bool {
	return strings.HasPrefix(id, prefix+"_") && ValidCommandID(id)
}
func MusicPlayerOperation(op string) bool {
	switch op {
	case "music.play", "music.pause", "music.next", "music.previous", "music.set_volume", "music.play_item":
		return true
	}
	return false
}
