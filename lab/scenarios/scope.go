package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type ScopeProbe func(context.Context, *world.World, world.Endpoint, contracts.Record) bool

var scopePaths = map[string]ScopeProbe{
	"query": func(ctx context.Context, w *world.World, endpoint world.Endpoint, record contracts.Record) bool {
		var result contracts.ReadResult
		_, err := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.message"}, &result)
		return err == nil && len(result.Records) == 0
	},
	"search": func(ctx context.Context, w *world.World, endpoint world.Endpoint, record contracts.Record) bool {
		var result contracts.ReadResult
		_, err := world.Request(ctx, endpoint, "/v1/op/search", contracts.ReadQuery{Text: "restrictedneedle"}, &result)
		return err == nil && len(result.Records) == 0
	},
	"records": func(ctx context.Context, w *world.World, endpoint world.Endpoint, record contracts.Record) bool {
		status, _ := world.Request(ctx, endpoint, "/v1/records/"+record.Collection+"/"+record.ID, nil, nil)
		return status == 404
	},
	"commands": func(ctx context.Context, w *world.World, endpoint world.Endpoint, record contracts.Record) bool {
		var result struct {
			Error struct {
				Code string `json:"code"`
			} `json:"error"`
		}
		id := contracts.NewCommandID()
		args, _ := json.Marshal(map[string]any{"chat_id": contracts.NewID("cht"), "text": "[ab-test " + id + "] scope probe"})
		command := contracts.Command{ID: id, Op: "messages.send", Args: args}
		status, _ := world.Request(ctx, endpoint, "/v1/op/messages.send", command, &result)
		return status == 401 || status == 403 || (status == 404) || (status == 422 && result.Error.Code == "unsupported")
	},
}

func RegisterScopePath(name string, probe ScopeProbe) { scopePaths[name] = probe }
func restrictedMessage(w *world.World, key string) bool {
	return w.ControlAction("/external-change", map[string]any{"key": key, "collection": "messages.message", "data": map[string]any{"title": "[ab-test] " + key, "text": "restrictedneedle", "optional_future": map[string]any{"counter": json.Number("9007199254740993")}}}) == nil
}
func scopeLeak(ctx context.Context, w *world.World, paths []string, add func(string, bool)) bool {
	admin := w.Gateway
	token, err := w.ScopedToken(admin, "admin")
	if err != nil {
		return false
	}
	admin.Token = token
	if !restrictedMessage(w, "restricted-before") {
		return false
	}
	var messages contracts.ReadResult
	if w.Await(10*time.Second, func() bool {
		_, err := world.Request(ctx, admin, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.message"}, &messages)
		return err == nil && len(messages.Records) == 1
	}) != nil {
		return false
	}
	record := messages.Records[0]
	if w.Await(10*time.Second, func() bool {
		report, err := w.Doctor(w.Replicas[0])
		records, _ := report["records"].(map[string]any)
		count, _ := records["messages.message"].(float64)
		return err == nil && count >= 1
	}) != nil {
		return false
	}
	add("replica_physically_contains_messages_before_scoped_probes", true)
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	observers := []*watcher{}
	for i, endpoint := range endpoints {
		for _, path := range paths {
			if path == "sse" {
				continue
			}
			probe, ok := scopePaths[path]
			if !ok {
				return false
			}
			if !probe(ctx, w, endpoint, record) {
				return false
			}
			add("scope_"+path+"_"+map[bool]string{true: "gateway", false: "replica"}[i == 0], true)
		}
		observer, err := watch(ctx, endpoint, false, func(record contracts.Record) bool { return truthAbsent(w, record) })
		if err != nil {
			return false
		}
		defer observer.close()
		observers = append(observers, observer)
	}
	if !restrictedMessage(w, "restricted-live") {
		return false
	}
	if w.ControlAction("/external-change", map[string]string{"key": "visible-scope-barrier"}) != nil {
		return false
	}
	if w.Await(10*time.Second, func() bool {
		for _, observer := range observers {
			if _, ok := observer.observed("visible-scope-barrier"); !ok {
				return false
			}
		}
		return true
	}) != nil {
		return false
	}
	for _, observer := range observers {
		observer.mu.Lock()
		leak := false
		for _, seen := range observer.changes {
			if seen.Change.Record.Source == contracts.Messages {
				leak = true
			}
		}
		observer.mu.Unlock()
		if leak {
			return false
		}
	}
	add("scope_sse_live_filtered_with_visible_barrier", true)
	return true
}
