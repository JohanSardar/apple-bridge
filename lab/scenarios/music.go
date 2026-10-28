package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func runMusicCatalog(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
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
	endpoint.Token, err = w.ScopedToken(endpoint, "music:read,system:read")
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
