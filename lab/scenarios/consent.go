package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"net/url"
	"strings"
	"time"
)

func runConsent(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	runAgentOAuth(ctx, w, c, e, add)
	_, err := w.GatewayCommand("agent", "list")
	if err != nil {
		add("authority", false)
		return
	}
	rawAgent, err := w.GatewayCommand("agent", "create", "--name", "Bridge Test consent", "--level", "approve", "--scope", "messages:write")
	var created struct {
		Token string `json:"token"`
	}
	if err == nil {
		err = json.Unmarshal(rawAgent, &created)
	}
	token := created.Token
	if err != nil {
		add("agent", false)
		return
	}
	add("synthetic_chat", w.ControlAction("/messages", map[string]any{"action": "seed", "count": 0}) == nil)
	var chats contracts.ReadResult
	err = w.Await(5*time.Second, func() bool {
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.chat"}, &chats)
		return err == nil && len(chats.Records) > 0
	})
	if err != nil {
		add("chat", false)
		return
	}
	args, _ := json.Marshal(map[string]any{"chat_id": chats.Records[0].ID, "text": "[ab-test consent] synthetic approval"})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 1}
	code, raw, _ := agentRequest(ctx, c.origin, token, "POST", "/v1/op/messages.send", command)
	var record contracts.CommandRecord
	json.Unmarshal(raw, &record)
	add("awaiting_approval", code == 200 && record.State == contracts.CommandAwaitingApproval)
	req, _ := http.NewRequestWithContext(ctx, "GET", c.origin+"/approvals", nil)
	req.AddCookie(c.cookie)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		add("approvals_page", false)
		return
	}
	body, _ := io.ReadAll(resp.Body)
	resp.Body.Close()
	add("owner_arguments", resp.StatusCode == 200 && bytes.Contains(body, []byte("synthetic approval")) && bytes.Contains(body, []byte("argument_hash")))
	values := url.Values{"decision": {"approve"}}
	req, _ = http.NewRequestWithContext(ctx, "POST", c.origin+"/approvals/"+command.ID, strings.NewReader(values.Encode()))
	req.AddCookie(c.cookie)
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Origin", c.origin)
	resp, err = http.DefaultClient.Do(req)
	if err != nil {
		add("approve_form", false)
		return
	}
	resp.Body.Close()
	add("approve_form", resp.StatusCode == 200)
	err = w.Await(10*time.Second, func() bool {
		_, raw, _ := agentRequest(ctx, c.origin, token, "POST", "/v1/op/commands.status", contracts.CommandQuery{ID: command.ID})
		json.Unmarshal(raw, &record)
		return record.State == contracts.CommandObserved
	})
	add("approved_observed", err == nil)
	for _, path := range []string{"/web/v1/session", "/web/v1/live", "/web/v1/handles", "/sw.js", "/manifest.webmanifest"} {
		code, _, _ := agentRequest(ctx, c.origin, "", "GET", path, nil)
		add("removed_"+path, code == 404)
	}
	code, raw, _ = agentRequest(ctx, c.origin, "", "GET", "/", nil)
	add("plain_landing", code == 200 && !bytes.Contains(raw, []byte("Solid")) && bytes.Contains(raw, []byte("Apple Bridge")))
}
