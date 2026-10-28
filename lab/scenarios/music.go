package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
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
	if s.ID == "MUS-CONTROL-REAL" {
		err = client.Call(ctx, "music.now_playing", contracts.Query{Args: json.RawMessage(`{}`)}, &out)
		e.Status = "limited"
		reason := "Control fixture qualification pending; Recently Played changes require prior-state restoration"
		if err != nil {
			reason = "Automation: dev.applebridge.helper → com.apple.Music missing; consent not requested"
		}
		e.Assertions = append(e.Assertions, Assertion{Name: "music_control_real", Status: "limited", Detail: reason})
		return
	}
	err = client.Call(ctx, "music.search_catalog", contracts.Query{Args: json.RawMessage(`{"term":"Here Comes the Sun Beatles","entity":"song","limit":5}`), DeadlineMS: 20000}, &out)
	add("catalog_query", err == nil)
	var result struct {
		Items []struct {
			ID string `json:"catalog_id"`
		} `json:"items"`
	}
	decode := json.Unmarshal(out.ResultJSON, &result)
	add("catalog_id", decode == nil && len(result.Items) > 0 && result.Items[0].ID != "")
	for _, args := range []string{`{"term":""}`, `{"term":"test","limit":101}`, `{"term":"test","url":"https://example.com"}`, `{"term":"test","entity":"playlist"}`, `{"term":"test","limit":0}`, `{"term":"test","limit":null}`, `{"term":"test","limit":1.5}`} {
		err = client.Call(ctx, "music.search_catalog", contracts.Query{Args: json.RawMessage(args)}, &out)
		var problem *contracts.Error
		add("invalid_rejected", errors.As(err, &problem) && problem.Code == "invalid")
	}
	if level == "sim" {
		for _, op := range []string{"set_volume", "play_item", "pause", "play", "next", "previous"} {
			args := json.RawMessage(`{}`)
			if op == "set_volume" {
				args = json.RawMessage(`{"volume":0}`)
			}
			if op == "play_item" {
				args = json.RawMessage(`{"id":"ABCDEF0123456789"}`)
			}
			var command contracts.CommandRecord
			commandID := contracts.NewCommandID()
			err = client.Call(ctx, "music."+op, contracts.Command{ID: commandID, Args: args, WaitMS: 15000}, &command)
			add("command_"+op, err == nil && command.State == "observed")
			if err != nil || command.State != "observed" {
				code := ""
				var problem *contracts.Error
				if errors.As(err, &problem) {
					code = problem.Code
				}
				var stored contracts.CommandRecord
				lookup := client.Call(ctx, "commands.status", contracts.CommandQuery{ID: commandID}, &stored)
				lookupCode := ""
				if errors.As(lookup, &problem) {
					lookupCode = problem.Code
				}
				e.Assertions[len(e.Assertions)-1].Detail = "state=" + string(command.State) + " code=" + code + " stored=" + string(stored.State) + " lookup=" + lookupCode
			}

			err = client.Call(ctx, "music.now_playing", contracts.Query{Args: json.RawMessage(`{}`)}, &out)
			var player struct {
				State  string `json:"state"`
				Volume *int   `json:"volume"`
			}
			decode = json.Unmarshal(out.ResultJSON, &player)
			add("readback_"+op, err == nil && decode == nil && player.Volume != nil && *player.Volume == 0 && player.State != "")
		}
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
