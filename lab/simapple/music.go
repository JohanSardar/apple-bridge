package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"strings"
	"time"
)

func simulateMusicQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "music.search_catalog" {
		return "", nil, false
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil {
		return "invalid", nil, true
	}
	for _, key := range []string{"term", "entity", "limit"} {
		if string(fields[key]) == "null" {
			return "invalid", nil, true
		}
	}
	var args struct {
		Term   string `json:"term"`
		Entity string `json:"entity"`
		Limit  *int   `json:"limit"`
	}
	if contracts.DecodeInput(raw, &args) != nil || strings.TrimSpace(args.Term) == "" || len(args.Term) > 512 || (args.Limit != nil && (*args.Limit < 1 || *args.Limit > 100)) || (args.Entity != "" && args.Entity != "song" && args.Entity != "album" && args.Entity != "musicArtist") {
		return "invalid", nil, true
	}
	entity := args.Entity
	if entity == "" {
		entity = "song"
	}
	result, _ := json.Marshal(map[string]any{"items": []any{map[string]string{"catalog_id": "1441164738", "name": "Fixture", "artist": "Fixture artist", "kind": entity}}})
	return "", result, true
}

type musicPlayer struct {
	State     string
	Volume    int
	ID        string
	Step      int
	Published bool
}

func (m *musicPlayer) player() map[string]any {
	state := m.State
	if state == "" {
		state = "stopped"
	}
	return map[string]any{"state": state, "volume": m.Volume, "id": m.ID, "position": 0, "track": "Fixture", "artist": "Fixture", "album": "Fixture"}
}
func (f *fakeApple) musicCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture, uint64) error, checkpoint func(contracts.Source) string) (bool, error) {
	f.mu.Lock()
	defer f.mu.Unlock()
	if frame.T == "Prepare" {
		if frame.Action == nil || !strings.HasPrefix(frame.Action.Op, "music.") {
			return false, nil
		}
		f.seedMusic()
		id := contracts.NewID("ticket")
		f.tickets[id] = ticket{CommandID: frame.CommandID, Action: *frame.Action, Hash: frame.RequestHash, Fence: fence, Expires: time.Now().Add(30 * time.Second)}
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: id, Fingerprint: frame.RequestHash})
		return true, err
	}
	if frame.T != "Dispatch" {
		return false, nil
	}
	t, ok := f.tickets[frame.Ticket]
	if !ok || !strings.HasPrefix(t.Action.Op, "music.") {
		return false, nil
	}
	delete(f.tickets, frame.Ticket)
	if t.CommandID != frame.CommandID || t.Fence != fence || !time.Now().Before(t.Expires) {
		_, err := wire.Send(contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: contracts.ErrExpired})
		return true, err
	}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, StartedAt: time.Now()}); err != nil {
		return true, err
	}
	raw, _ := json.Marshal(t.Action.Args)
	var args contracts.MusicCommandArgs
	json.Unmarshal(raw, &args)
	switch t.Action.Op {
	case "music.play":
		f.music.State = "playing"
	case "music.pause":
		f.music.State = "paused"
	case "music.play_item":
		f.music.ID = args.ID
		f.music.State = "playing"
	case "music.set_volume":
		if args.Volume == nil {
			return true, contracts.ErrInvalid
		}
		f.music.Volume = *args.Volume
	case "music.favorite", "music.add_to_playlist":
		item := f.records["music-item"]
		playlist := f.records["music-playlist"]
		if args.ID != "ABCDEF0123456789" || (t.Action.Op == "music.add_to_playlist" && args.PlaylistID != "DEF0123456789ABC") {
			return true, contracts.ErrInvalid
		}
		var value map[string]any
		if t.Action.Op == "music.favorite" {
			json.Unmarshal(item.Data, &value)
			value["favorite"] = args.Favorite
			item.Data, _ = json.Marshal(value)
			item.ObservedAt = time.Now().UTC()
			f.records["music-item"] = item
		} else {
			json.Unmarshal(playlist.Data, &value)
			items, _ := value["items"].([]any)
			value["items"] = append(items, map[string]any{"$ref": map[string]string{"collection": "music.library_item", "native_key": args.ID}})
			value["count"] = len(items) + 1
			playlist.Data, _ = json.Marshal(value)
			playlist.ObservedAt = time.Now().UTC()
			f.records["music-playlist"] = playlist
		}
		capture := contracts.Capture{Source: "music", Observations: []contracts.Observation{f.records["music-item"], f.records["music-playlist"]}}
		f.mu.Unlock()
		emitErr := emit(capture, 0)
		f.mu.Lock()
		if emitErr != nil {
			return true, emitErr
		}
		f.music.Published = true
	case "music.next", "music.previous":
		f.music.Step++
		if f.music.Step%2 == 0 {
			f.music.ID = "ABCDEF0123456789"
		} else {
			f.music.ID = "ABCDEF0123456780"
		}
	}
	if !f.music.Published {
		capture := contracts.Capture{Source: "music", Observations: []contracts.Observation{f.records["music-item"], f.records["music-playlist"]}}
		f.mu.Unlock()
		emitErr := emit(capture, 0)
		f.mu.Lock()
		if emitErr != nil {
			return true, emitErr
		}
		f.music.Published = true
	}
	details := map[string]any{"verified": true, "fingerprint": t.Hash, "player": map[string]any{"state": f.music.player()["state"], "volume": f.music.Volume}}
	if t.Action.Op == "music.favorite" || t.Action.Op == "music.add_to_playlist" {
		key := args.ID
		if t.Action.Op == "music.add_to_playlist" {
			key = args.PlaylistID
		}
		details["observed"] = true
		details["native_key"] = key
		details["capture_checkpoint"] = checkpoint("music")
	}
	evidence, _ := json.Marshal(details)
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: frame.CommandID, Disposition: "locally_observed", Evidence: evidence}
	if err := f.guard(guardEntry{CommandID: frame.CommandID, Ticket: frame.Ticket, Outcome: &outcome}); err != nil {
		return true, err
	}
	_, err := wire.Send(outcome)
	return true, err
}
func init() { sourceCommands = append(sourceCommands, (*fakeApple).musicCommand) }

func (f *fakeApple) seedMusic() {
	if _, ok := f.records["music-item"]; ok {
		return
	}
	f.records["music-item"] = contracts.Observation{Kind: "upsert", Collection: "music.library_item", NativeKey: "ABCDEF0123456789", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"name":"Bridge Test track","artist":"Fixture","album":"Fixture","favorite":false}`)}
	f.records["music-playlist"] = contracts.Observation{Kind: "upsert", Collection: "music.playlist", NativeKey: "DEF0123456789ABC", ObservedAt: time.Now().UTC(), Data: json.RawMessage(`{"name":"Bridge Test playlist","items":[],"count":0}`)}
}
