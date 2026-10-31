package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"net/http"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
)

func runWebMessagesRead(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Metrics: map[string]any{"writes_submitted": 0}}
	if level == "real" {
		return runNativeMessagesCommands(ctx, s, level, e)
	}
	w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "messages:read,system:read"})
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "sim_world", Passed: false})
		return e
	}
	defer w.Close()
	if w.ControlAction("/messages", map[string]any{"action": "seed", "count": 1}) != nil || w.FlushCaptures(15*time.Second) != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "sim_capture", Passed: false})
		return e
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		e.Assertions = append(e.Assertions, Assertion{Name: "sim_aliases", Passed: false})
		return e
	}
	return proveWebSelfRead(ctx, e, w, aliases[aliasKey(contracts.Messages, "messages.chat", "message-chat")], aliases[aliasKey(contracts.Messages, "messages.message", "message-000000")], "self@example.com", "[ab-test] Unicode 🧪 שלום e\u0301", true)
}

func runNativeWebMessagesRead(ctx context.Context, e Evidence, w *world.World, native, state *sql.DB, rawChat, chatID string) Evidence {
	var guid, text, peer string
	var outgoing bool
	err := native.QueryRowContext(ctx, `SELECT m.guid,m.text,m.is_from_me,h.id FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id JOIN chat_handle_join p ON p.chat_id=c.ROWID JOIN handle h ON h.ROWID=p.handle_id WHERE c.guid=? AND m.item_type=0 AND COALESCE(m.associated_message_type,0)=0 AND COALESCE(m.date_retracted,0)=0 AND length(m.text)>0 AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID) ORDER BY m.ROWID DESC LIMIT 1`, rawChat).Scan(&guid, &text, &outgoing, &peer)
	e.Assertions = append(e.Assertions, Assertion{Name: "independent_visible_self_message", Passed: err == nil})
	if err != nil {
		return e
	}
	var messageID string
	err = w.Await(30*time.Second, func() bool {
		return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=?", guid).Scan(&messageID) == nil
	})
	e.Assertions = append(e.Assertions, Assertion{Name: "native_self_message_captured", Passed: err == nil})
	if err != nil {
		return e
	}
	return proveWebSelfRead(ctx, e, w, chatID, messageID, peer, text, outgoing)
}

func proveWebSelfRead(ctx context.Context, e Evidence, w *world.World, chatID, messageID, peer, text string, outgoing bool) Evidence {
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	add("self_chat_and_message_identity", chatID != "" && messageID != "" && peer != "")
	if chatID == "" || messageID == "" || peer == "" {
		return e
	}
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("private_web_authority", false)
		return e
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	add("private_web_session", err == nil)
	if err != nil {
		return e
	}
	bounded, cancel := context.WithTimeout(ctx, 30*time.Second)
	defer cancel()
	client := &webClient{ctx: bounded, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
	conn, err := client.socket()
	add("authenticated_web_socket", err == nil)
	if err != nil {
		return e
	}
	defer conn.CloseNow()
	for _, view := range []string{"chats", "thread"} {
		args := map[string]any{"participants": []string{peer}, "limit": 200}
		if view == "thread" {
			args = map[string]any{"chat_id": chatID, "anchor": messageID, "limit": 1}
		}
		err = wsjson.Write(bounded, conn, map[string]any{"t": "sub", "id": view, "view": view, "args": args})
		found := false
		for tries := 0; err == nil && tries < 12; tries++ {
			var frame struct {
				Type string          `json:"t"`
				Sub  string          `json:"sub"`
				Data json.RawMessage `json:"data"`
			}
			err = wsjson.Read(bounded, conn, &frame)
			if err != nil || frame.Type == "error" {
				break
			}
			if frame.Type != "snap" || frame.Sub != view {
				continue
			}
			if view == "chats" {
				var result contracts.ChatsResult
				if json.Unmarshal(frame.Data, &result) == nil {
					for _, chat := range result.Chats {
						found = found || chat.Chat.ID == chatID
					}
				}
			} else {
				var result contracts.ThreadResult
				if json.Unmarshal(frame.Data, &result) == nil && len(result.Items) == 1 {
					var data struct {
						ChatID   string `json:"chat_id"`
						Text     string `json:"text"`
						Outgoing bool   `json:"is_from_me"`
					}
					item := result.Items[0]
					found = json.Unmarshal(item.Data, &data) == nil && item.ID == messageID && !item.Deleted && data.ChatID == chatID && data.Text == text && data.Outgoing == outgoing
				}
			}
			break
		}
		add("web_"+view+"_matches_independent_self_truth", found)
		if !found {
			return e
		}
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
