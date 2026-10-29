package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os/exec"
	"strings"
	"time"
)

func runMusicCatalog(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(n string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: n, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	if s.ID == "MUS-IDLE" {
		idle := s
		idle.ID = "MAC-IDLE"
		e = runMac(ctx, idle, level)
		e.Scenario = s.ID
		for i := range e.Assertions {
			if e.Assertions[i].Name == "helper_cpu_under_two_percent" {
				e.Assertions[i].Name = "whole_helper_idle_cpu"
			}
		}
		return
	}
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewReal(ctx, level, 0)
	} else {
		w, err = world.New(ctx, level, 1, 0)
	}
	add("world_started", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "music:read,music:write,system:read")
	add("scopes", err == nil)
	if err != nil {
		return
	}
	client := scripted.Client{Endpoint: endpoint}
	if level == "real" {
		err = w.Await(20*time.Second, func() bool {
			var out contracts.QueryResult
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
		})
		add("helper_ready", err == nil)
		if err != nil {
			return
		}
	}
	var out contracts.QueryResult
	if s.ID == "MUS-CONTROL-REAL" || s.ID == "MUS-PLAYLIST-REAL" {
		runMusicPlaybackProof(ctx, w, s, &e)
		return
	}
	err = client.Call(ctx, "music.search_catalog", contracts.Query{Args: json.RawMessage(`{"term":"Here Comes the Sun Beatles","entity":"song","limit":5}`), DeadlineMS: 20000}, &out)
	add("catalog_query", err == nil)
	var result struct {
		Items []struct {
			ID     string `json:"catalog_id"`
			Name   string `json:"name"`
			Artist string `json:"artist"`
		} `json:"items"`
	}
	decode := json.Unmarshal(out.ResultJSON, &result)
	add("catalog_id", decode == nil && len(result.Items) > 0 && result.Items[0].ID != "")
	if level == "real" {
		matched := false
		for _, item := range result.Items {
			matched = matched || (item.ID != "" && strings.Contains(strings.ToLower(item.Name), "here comes the sun") && strings.Contains(strings.ToLower(item.Artist), "beatles"))
		}
		add("well_known_song_catalog_id", matched)
	}
	for _, args := range []string{`{"term":""}`, `{"term":"test","limit":101}`, `{"term":"test","url":"https://example.com"}`, `{"term":"test","entity":"playlist"}`, `{"term":"test","limit":0}`, `{"term":"test","limit":null}`, `{"term":"test","limit":1.5}`} {
		err = client.Call(ctx, "music.search_catalog", contracts.Query{Args: json.RawMessage(args)}, &out)
		var problem *contracts.Error
		add("invalid_rejected", errors.As(err, &problem) && problem.Code == "invalid")
	}
	for _, ms := range []int{-1, 60001} {
		err = client.Call(ctx, "music.search_catalog", contracts.Query{Args: json.RawMessage(`{"term":"fixture"}`), DeadlineMS: ms}, &out)
		var problem *contracts.Error
		add("query_deadline_bounds", errors.As(err, &problem) && problem.Code == "invalid")
	}

	if level == "sim" {
		var itemID, playlistID string
		for _, op := range []string{"set_volume", "play_item", "pause", "play", "next", "previous"} {
			args := json.RawMessage(`{}`)
			if op == "set_volume" {
				args = json.RawMessage(`{"volume":0}`)
			}
			if op == "play_item" {
				args, _ = json.Marshal(map[string]string{"id": itemID})
			}
			var command contracts.CommandRecord
			err = client.Call(ctx, "music."+op, contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &command)
			add("command_"+op, err == nil && command.State == "observed")
			add("command_native_identity_private_"+op, !strings.Contains(string(command.Evidence), "ABCDEF0123456789"))
			if op == "set_volume" {
				for _, target := range []struct {
					Collection, Prefix string
					ID                 *string
				}{{"music.library_item", "mli_", &itemID}, {"music.playlist", "mpl_", &playlistID}} {
					var captured contracts.ReadResult
					err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: target.Collection, Limit: 10}, &captured)
					valid := err == nil && len(captured.Records) == 1
					if valid {
						*target.ID = captured.Records[0].ID
						valid = strings.HasPrefix(*target.ID, target.Prefix) && !strings.Contains(string(captured.Records[0].Data), "persistent_id")
					}
					add("bridge_identity_"+target.Collection, valid)
				}
			}
			err = client.Call(ctx, "music.now_playing", contracts.Query{Args: json.RawMessage(`{}`)}, &out)
			var player struct {
				State  string `json:"state"`
				Volume *int   `json:"volume"`
			}
			decode = json.Unmarshal(out.ResultJSON, &player)
			var fields map[string]json.RawMessage
			json.Unmarshal(out.ResultJSON, &fields)
			add("readback_"+op, err == nil && decode == nil && player.Volume != nil && *player.Volume == 0 && player.State != "" && fields["id"] == nil)
		}
		for _, op := range []string{"favorite", "add_to_playlist"} {
			args, _ := json.Marshal(map[string]any{"id": itemID, "favorite": true})
			targetID, collection := itemID, "music.library_item"
			if op == "add_to_playlist" {
				args, _ = json.Marshal(map[string]string{"id": itemID, "playlist_id": playlistID})
				targetID, collection = playlistID, "music.playlist"
			}
			count := 1
			if op == "add_to_playlist" {
				count = 2
			}
			for i := 0; i < count; i++ {
				var command contracts.CommandRecord
				err = client.Call(ctx, "music."+op, contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &command)
				add("library_command_"+op, err == nil && command.State == "observed" && command.CommitSeq > 0 && len(command.ObservedRecordIDs) == 1 && command.ObservedRecordIDs[0] == targetID)
				add("library_native_identity_private_"+op, !strings.Contains(string(command.Evidence), "ABCDEF0123456789") && !strings.Contains(string(command.Evidence), "DEF0123456789ABC"))
			}
			var captured contracts.ReadResult
			err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, Limit: 10}, &captured)
			valid := err == nil && len(captured.Records) == 1
			if valid {
				var data struct {
					Favorite bool  `json:"favorite"`
					Count    int   `json:"count"`
					Items    []any `json:"items"`
				}
				valid = json.Unmarshal(captured.Records[0].Data, &data) == nil && !strings.Contains(string(captured.Records[0].Data), "native_key")
				if op == "favorite" {
					valid = valid && data.Favorite
				} else {
					valid = valid && data.Count == 2 && len(data.Items) == 2
				}
			}
			add("captured_readback_"+op, valid)
		}
		for op, args := range map[string]map[string]any{"set_volume": {"volume": 101}, "play_item": {"id": "ABCDEF0123456789"}, "favorite": {"id": itemID, "favorite": "yes"}, "add_to_playlist": {"id": itemID, "playlist_id": "DEF0123456789ABC"}, "pause": {"unexpected": true}} {
			raw, _ := json.Marshal(args)
			var command contracts.CommandRecord
			err = client.Call(ctx, "music."+op, contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &command)
			var problem *contracts.Error
			add("invalid_command_"+op, errors.As(err, &problem) && problem.Code == "invalid")
		}
		store, inspectErr := state.InspectHome(w.Gateway.Home)
		shapeOK := false
		if inspectErr == nil {
			shape, shapeErr := store.ShapeViolations(ctx)
			shapeOK = shapeErr == nil && shape["music.library_item"] == 0 && shape["music.playlist"] == 0
			store.Close()
		}
		add("doctor_music_shapes", shapeOK)
		for _, entity := range []string{"song", "album", "musicArtist"} {
			args, _ := json.Marshal(map[string]any{"term": "fixture", "entity": entity, "limit": 1})
			err = client.Call(ctx, "music.search_catalog", contracts.Query{Args: args}, &out)
			var typed struct {
				Items []struct {
					Kind string `json:"kind"`
					ID   string `json:"catalog_id"`
				} `json:"items"`
			}
			decode = json.Unmarshal(out.ResultJSON, &typed)
			add("entity_"+entity, err == nil && decode == nil && len(typed.Items) == 1 && typed.Items[0].Kind == entity && typed.Items[0].ID != "")
		}
	}

	return
}
