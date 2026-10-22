package scenarios

import (
	"context"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type threadFixture struct {
	chat      string
	otherChat string
	root      string
	replies   []string
}

func populateThreadFixture(ctx context.Context, w *world.World, huge bool) (threadFixture, error) {
	dataset := demo.Generate(demo.Options{Seed: 1, Huge: huge})
	root := rmObservation("messages.message", "transcript-originator-root", map[string]any{
		"chat_id": rmRef("messages.chat", "demo-chat-39"), "sent_at": demo.Epoch.AddDate(0, -12, 0), "kind": "message", "text": "transcriptoriginatorfixture root", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": "transcriptoriginatorfixture root"}},
	})
	dataset.Records = append(dataset.Records, root)
	if !huge {
		for offset := -325; offset <= 325; offset++ {
			if offset == 0 {
				continue
			}
			body := "[ab-test transcript] bounded originator gap"
			dataset.Records = append(dataset.Records, rmObservation("messages.message", fmt.Sprintf("transcript-originator-gap-%03d", offset+325), map[string]any{
				"chat_id": rmRef("messages.chat", "demo-chat-39"), "sent_at": demo.Epoch.AddDate(0, -12, 0).Add(time.Duration(offset) * time.Minute), "kind": "message", "text": body, "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}},
			}))
		}
	}
	for index := 0; index < 205; index++ {
		dataset.Records = append(dataset.Records, rmObservation("messages.message", fmt.Sprintf("transcript-originator-reply-%03d", index), map[string]any{
			"chat_id": rmRef("messages.chat", "demo-chat-39"), "thread_originator_id": rmRef("messages.message", "transcript-originator-root"), "sent_at": demo.Epoch.Add(time.Duration(index) * time.Second), "kind": "message", "text": "transcriptoriginatorfixture distant reply", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": "transcriptoriginatorfixture distant reply"}},
		}))
	}
	for index := range dataset.Records {
		if dataset.Records[index].ObservedAt.IsZero() {
			dataset.Records[index].ObservedAt = demo.Epoch
		}
	}
	previousTimeout := w.Control.Timeout
	w.Control.Timeout = 3 * time.Minute
	defer func() { w.Control.Timeout = previousTimeout }()
	if err := w.ControlAction("/dataset", dataset.Dataset); err != nil {
		return threadFixture{}, fmt.Errorf("populate: %w", err)
	}
	ids, err := demo.Fixtures(ctx, w)
	if err != nil {
		return threadFixture{}, err
	}
	var records contracts.ReadResult
	if err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.message", Text: "transcriptoriginatorfixture", IncludeAliases: true, Limit: 1000}, &records); err != nil {
		return threadFixture{}, err
	}
	aliases := map[string]string{}
	for _, alias := range records.Aliases {
		aliases[alias.NativeKey] = alias.RecordID
	}
	fixture := threadFixture{chat: ids["demo-chat-39"], otherChat: ids["message-chat"], root: aliases["transcript-originator-root"]}
	for index := 0; index < 205; index++ {
		fixture.replies = append(fixture.replies, aliases[fmt.Sprintf("transcript-originator-reply-%03d", index)])
	}
	if fixture.chat == "" || fixture.root == "" || len(records.Records) != 206 {
		return fixture, fmt.Errorf("thread fixture incomplete")
	}
	return fixture, nil
}
