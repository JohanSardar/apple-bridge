package scenarios

import (
	"context"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
)

func runIteratorPool(ctx context.Context, w *world.World, trusted *webClient, auth *webAuthenticator, evidence *Evidence, add func(string, bool)) {
	bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	var sources contracts.ReadResult
	err := (scripted.Client{Endpoint: w.Gateway}).Call(bounded, "sources.status", struct{}{}, &sources)
	var sourceID string
	for _, source := range sources.Records {
		if source.Source == contracts.Messages {
			sourceID = source.ID
		}
	}
	add("source_identity", err == nil && sourceID != "")
	if err != nil || sourceID == "" {
		return
	}
	public := &webClient{ctx: bounded, origin: trusted.origin}
	status, _, err := public.signIn(auth, "login", "", "public")
	add("independent_public_session", err == nil && status == 200)
	if err != nil || status != 200 {
		return
	}
	var sockets []*websocket.Conn
	defer func() {
		for _, socket := range sockets {
			socket.CloseNow()
		}
	}()
	for _, client := range []*webClient{trusted, public} {
		copy := *client
		copy.ctx = bounded
		socket, problem := copy.socket()
		add("live_session", problem == nil)
		if problem != nil {
			return
		}
		sockets = append(sockets, socket)
	}
	const perSession = 16
	for index, socket := range sockets {
		for sub := 0; sub < perSession; sub++ {
			if wsjson.Write(bounded, socket, map[string]any{"t": "sub", "id": fmt.Sprintf("pool-%d-%d", index, sub), "view": "record", "args": map[string]any{"collection": "system.source", "id": sourceID}}) != nil {
				add("subscriptions_registered", false)
				return
			}
		}
	}
	for _, socket := range sockets {
		seen := map[string]bool{}
		for len(seen) < perSession {
			var frame struct {
				Type string `json:"t"`
				Sub  string `json:"sub"`
			}
			if wsjson.Read(bounded, socket, &frame) != nil {
				add("all_32_snapshots_progress_with_pool_8", false)
				return
			}
			if frame.Type == "snap" {
				seen[frame.Sub] = true
			}
		}
	}
	add("all_32_snapshots_progress_with_pool_8", true)
	started := time.Now()
	add("source_change_queued", w.ControlAction("/messages", map[string]any{"action": "capability-unproven"}) == nil)
	for _, socket := range sockets {
		seen := map[string]bool{}
		for len(seen) < perSession {
			var frame struct {
				Type    string             `json:"t"`
				Sub     string             `json:"sub"`
				Changes []contracts.Change `json:"changes"`
			}
			if wsjson.Read(bounded, socket, &frame) != nil {
				add("all_32_iterators_observe_next_commit", false)
				return
			}
			for _, change := range frame.Changes {
				if change.Record.ID == sourceID {
					seen[frame.Sub] = true
				}
			}
		}
	}
	add("all_32_iterators_observe_next_commit", true)
	evidence.Latency["32_live_subscriptions_commit"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	err = (scripted.Client{Endpoint: w.Gateway}).Call(bounded, "sources.status", struct{}{}, &sources)
	add("reads_progress_while_iterators_wait", err == nil && len(sources.Records) > 0)
}
