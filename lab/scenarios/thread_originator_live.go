package scenarios

import (
	"context"
	"fmt"
	"net/url"
	"os"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
)

type originatorFrame struct {
	Type string `json:"t"`
	Data struct {
		Items []contracts.ThreadItem `json:"items"`
		Older uint64                 `json:"older"`
		Newer uint64                 `json:"newer"`
	} `json:"data"`
	Changes []struct {
		Op  string               `json:"op"`
		ID  string               `json:"id"`
		Row contracts.ThreadItem `json:"row"`
	} `json:"changes"`
	Older uint64 `json:"older"`
	Newer uint64 `json:"newer"`
}

func threadWebClient(ctx context.Context, w *world.World) (*webClient, error) {
	_, port, _ := strings.Cut(w.Gateway.Host, ":")
	client := &webClient{ctx: ctx, origin: "http://localhost:" + port}
	output, err := exec.CommandContext(ctx, "bin/bridge", "web", "enroll", "--home", w.Gateway.Home, "--host", client.origin).Output()
	if err != nil {
		return nil, err
	}
	link, err := url.Parse(strings.TrimSpace(string(output)))
	if err != nil {
		return nil, err
	}
	auth, err := newWebAuthenticator()
	if err != nil {
		return nil, err
	}
	status, _, err := client.signIn(auth, "enroll", link.Fragment, "trusted")
	if err != nil {
		return nil, err
	}
	if status != 200 {
		return nil, fmt.Errorf("thread enrollment status %d", status)
	}
	return client, nil
}

func checkThreadOriginatorLive(ctx context.Context, w *world.World, fixture threadFixture, add func(string, bool)) {
	ctx, cancel := context.WithTimeout(ctx, 20*time.Second)
	defer cancel()
	client, err := threadWebClient(ctx, w)
	add("filtered_live_session", err == nil)
	if err != nil {
		return
	}
	connection, err := client.socket()
	add("filtered_live_web_1_socket", err == nil)
	if err != nil {
		return
	}
	defer connection.CloseNow()
	connection.SetReadLimit(1 << 20)
	err = wsjson.Write(ctx, connection, map[string]any{"t": "sub", "id": "originator", "view": "thread", "args": map[string]any{"chat_id": fixture.chat, "thread_originator_id": fixture.root, "anchor": "latest", "limit": 200}})
	add("filtered_live_subscribe", err == nil)
	if err != nil {
		return
	}
	read := func(predicate func(originatorFrame) bool) (originatorFrame, error) {
		for {
			var frame originatorFrame
			if err := wsjson.Read(ctx, connection, &frame); err != nil {
				return frame, err
			}
			if predicate(frame) {
				return frame, nil
			}
		}
	}
	snapshot, err := read(func(frame originatorFrame) bool { return frame.Type == "snap" || frame.Type == "error" })
	if err != nil {
		fmt.Fprintf(os.Stderr, "originator snapshot: %v\n", err)
	}
	add("filtered_live_snapshot_scoped_counts", err == nil && snapshot.Type == "snap" && len(snapshot.Data.Items) == 200 && snapshot.Data.Older == 6 && snapshot.Data.Newer == 0)
	if err != nil || snapshot.Type != "snap" {
		return
	}
	put := func(key, body string, originator bool) error {
		data := map[string]any{"chat_id": rmRef("messages.chat", "demo-chat-39"), "sent_at": demo.Epoch.Add(time.Hour), "text": body, "kind": "message", "parts": []any{map[string]any{"index": 0, "kind": "text", "text": body}}}
		if originator {
			data["thread_originator_id"] = rmRef("messages.message", "transcript-originator-root")
		}
		return rmPut(w, rmObservation("messages.message", key, data))
	}
	err = put("transcript-originator-unrelated", "An unrelated synthetic message", false)
	unrelated, readErr := read(func(frame originatorFrame) bool {
		for _, change := range frame.Changes {
			if change.Op == "window" {
				return true
			}
		}
		return frame.Type == "error"
	})
	valid := err == nil && readErr == nil && unrelated.Type == "delta" && unrelated.Older == 6 && unrelated.Newer == 0
	for _, change := range unrelated.Changes {
		valid = valid && change.Op == "window"
	}
	add("unrelated_live_message_excluded", valid)
	err = put("transcript-originator-live", "transcriptoriginatorlivefixture", true)
	inserted, readErr := read(func(frame originatorFrame) bool {
		for _, change := range frame.Changes {
			if rmData(change.Row.Record)["text"] == "transcriptoriginatorlivefixture" {
				return true
			}
		}
		return frame.Type == "error"
	})
	valid = err == nil && readErr == nil && inserted.Type == "delta" && inserted.Older == 7 && inserted.Newer == 0
	for _, change := range inserted.Changes {
		if rmData(change.Row.Record)["text"] == "transcriptoriginatorlivefixture" {
			valid = valid && change.Op == "upsert" && rmData(change.Row.Record)["thread_originator_id"] == fixture.root
		}
	}
	add("distant_live_reply_inserted_with_filtered_counts", valid)
	err = rmPut(w, rmObservation("messages.message", "transcript-originator-reply-204", map[string]any{"chat_id": rmRef("messages.chat", "demo-chat-39"), "sent_at": demo.Epoch.Add(204 * time.Second), "kind": "message", "text": "transcriptoriginatorfixture distant reply"}))
	removed, readErr := read(func(frame originatorFrame) bool {
		for _, change := range frame.Changes {
			if change.Op == "remove" && change.ID == fixture.replies[204] {
				return true
			}
		}
		return frame.Type == "error"
	})
	add("reassigned_live_reply_removed", err == nil && readErr == nil && removed.Type == "delta" && removed.Older == 6 && removed.Newer == 0)
}
