package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"time"
)

func runAgentActivityCapture(ctx context.Context, w *world.World, authority *trust.Authority, add func(string, bool)) {
	add("activity_capture_fixture", w.ControlAction("/messages", map[string]any{"action": "seed", "count": 0}) == nil)
	_, token, err := authority.CreateAgent("Bridge Test concurrent activity", trust.AgentFull, []contracts.Grant{"messages:write"})
	add("activity_capture_agent", err == nil)
	principal := w.Gateway
	principal.Token = token
	var chats contracts.ReadResult
	err = w.Await(5*time.Second, func() bool {
		code, err := world.Request(ctx, principal, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.chat"}, &chats)
		return err == nil && code == 200 && len(chats.Records) == 1
	})
	if err != nil {
		add("activity_capture_chat", false)
		return
	}
	readsCtx, stop := context.WithCancel(ctx)
	done := make(chan bool, 3)
	ready := make(chan struct{}, 3)
	for reader := 0; reader < 3; reader++ {
		_, readToken, readErr := authority.CreateAgent("Bridge Test concurrent reader", trust.AgentRead, []contracts.Grant{"messages:read"})
		if readErr != nil {
			stop()
			add("activity_capture_read_agent", false)
			return
		}
		readerPrincipal := principal
		readerPrincipal.Token = readToken
		go func() {
			ok := true
			ready <- struct{}{}
			for readsCtx.Err() == nil {
				var out contracts.ReadResult
				code, err := world.Request(readsCtx, readerPrincipal, "/v1/op/messages.chats", map[string]any{}, &out)
				if readsCtx.Err() == nil && (err != nil || code != 200) {
					ok = false
				}
				select {
				case <-readsCtx.Done():
				case <-time.After(10 * time.Millisecond):
				}
			}
			done <- ok
		}()
	}
	for reader := 0; reader < 3; reader++ {
		<-ready
	}
	observed := true
	for i := 0; i < 24; i++ {
		args, _ := json.Marshal(map[string]any{"chat_id": chats.Records[0].ID, "text": "[ab-test activity] concurrent capture"})
		command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
		var row contracts.CommandRecord
		code, err := world.Request(ctx, principal, "/v1/op/messages.send", command, &row)
		effects, effectErr := w.Effects()
		if err != nil || code != 200 || row.State != contracts.CommandObserved || row.CommitSeq == 0 || effectErr != nil || effects[command.ID] != 1 {
			observed = false
			break
		}
	}
	stop()
	readOK := true
	for reader := 0; reader < 3; reader++ {
		readOK = <-done && readOK
	}
	add("activity_concurrent_reads_succeed", readOK)
	add("activity_concurrent_sends_observed_once", observed)
}
