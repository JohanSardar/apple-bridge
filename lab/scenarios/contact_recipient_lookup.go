package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os/exec"
	"reflect"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRecipientLookup(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	if level != "sim" {
		add("demo_sim_only", false)
		return
	}
	w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "admin"})
	if err != nil {
		add("world_start", false)
		return
	}
	defer w.Close()
	if demo.Populate(w, demo.Generate(demo.Options{Seed: 1})) != nil {
		add("demo_dataset", false)
		return
	}
	if w.FlushCaptures(20*time.Second) != nil {
		add("demo_capture", false)
		return
	}
	queries := []struct {
		name, q      string
		limit, count int
	}{
		{"name", "Aster00", 8, 1}, {"name_with_digits", "Beryl01", 8, 1}, {"partial_email", "person01@", 8, 1}, {"normalized_phone", "5550101", 8, 1},
		{"bounded", "Demo", 3, 3}, {"capped", "Demo", 500, 25}, {"default", "Demo", 0, 25},
	}
	for _, query := range queries {
		var prior []string
		for _, protocol := range []string{"rest", "mcp"} {
			client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
			var result contracts.ReadResult
			err := client.Call(ctx, "contacts.lookup", map[string]any{"q": query.q, "limit": query.limit}, &result)
			add(query.name+"_"+protocol+"_count", err == nil && len(result.Records) == query.count)
			ids := []string{}
			names := []string{}
			for _, row := range result.Records {
				ids = append(ids, row.ID)
				var data map[string]any
				json.Unmarshal(row.Data, &data)
				names = append(names, data["given_name"].(string))
			}
			if query.name == "bounded" {
				add(protocol+"_deterministic_name_order", reflect.DeepEqual(names, []string{"Demo Aster00", "Demo Aster12", "Demo Aster24"}))
			}
			if protocol == "mcp" {
				add(query.name+"_rest_mcp_parity", err == nil && reflect.DeepEqual(ids, prior))
			}
			prior = ids
		}
	}
	client := scripted.Client{Endpoint: w.Gateway}
	var result contracts.ReadResult
	err = client.Call(ctx, "contacts.lookup", map[string]any{"name": "Aster00"}, &result)
	add("legacy_name_query", err == nil && len(result.Records) == 1)
	err = client.Call(ctx, "contacts.lookup", map[string]any{"name": "Demo"}, &result)
	add("legacy_name_cardinality", err == nil && len(result.Records) == 60)
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		for index, input := range []map[string]any{{"q": strings.Repeat("x", 65)}, {"q": "Demo", "limit": -1}, {"q": ""}, {"q": "   "}} {
			err = client.Call(ctx, "contacts.lookup", input, &result)
			add(fmt.Sprintf("invalid_lookup_%d_%s_rejected", index, protocol), err != nil)
		}
	}
	var replica *world.Endpoint
	rate, err := webLookupRate(ctx, w, &replica)
	add("lookup_rate_http_fixture", err == nil)
	if err != nil {
		return
	}
	statuses, _ := rate["statuses"].([]int)
	allowed := len(statuses) == 21
	for index, status := range statuses {
		allowed = allowed && (index < 20 && status == 200 || index == 20 && status == 429)
	}
	add("lookup_20_per_server_second_then_429", allowed)
	retry, _ := strconv.Atoi(rate["retry_after"].(string))
	add("lookup_rate_retry_after", retry > 0)
	add("lookup_rate_exact_second_reset", rate["after_reset"] == 200)
	add("lookup_rate_session_isolation", rate["isolated_session"] == 200)
	return
}
