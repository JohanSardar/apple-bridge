package scenarios

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
	"time"
)

type messageUnreadState struct {
	unread map[string]bool
	count  int
}

type messageUnreadBaseline map[string]messageUnreadState

func (before messageUnreadBaseline) matches(after messageUnreadBaseline) bool {
	chats := map[string]bool{}
	for chat := range before {
		chats[chat] = true
	}
	for chat := range after {
		chats[chat] = true
	}
	for chat := range chats {
		first, last := before[chat], after[chat]
		identical := len(first.unread) == len(last.unread)
		for guid := range first.unread {
			identical = identical && last.unread[guid]
		}
		if !identical && first.count != last.count {
			return false
		}
	}
	return true
}

func captureMessagesUnread(ctx context.Context, path string) (messageUnreadBaseline, error) {
	db, err := sql.Open("sqlite", "file:"+path+"?mode=ro")
	if err != nil {
		return nil, err
	}
	defer db.Close()
	rows, err := db.QueryContext(ctx, "SELECT c.guid,m.guid,COALESCE(m.is_read,0),CASE WHEN m.date>COALESCE(c.last_read_message_timestamp,0) THEN 1 ELSE 0 END FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.is_from_me=0 AND (COALESCE(m.is_read,0)=0 OR m.date>COALESCE(c.last_read_message_timestamp,0)) AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.chat_id=c.ROWID AND r.message_id=m.ROWID)")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	baseline := messageUnreadBaseline{}
	for rows.Next() {
		var chat, guid string
		var read, unread int
		if err := rows.Scan(&chat, &guid, &read, &unread); err != nil {
			return nil, err
		}
		state := baseline[chat]
		if state.unread == nil {
			state.unread = map[string]bool{}
		}
		if read == 0 {
			state.unread[guid] = true
		}
		state.count += unread
		baseline[chat] = state
	}
	return baseline, rows.Err()
}

func messagesUnreadPath() string {
	return filepath.Join(os.Getenv("HOME"), "Library", "Messages", "chat.db")
}

func finishMessagesUnreadAudit(before messageUnreadBaseline, result *Evidence) {
	ctx, cancel := context.WithTimeout(context.Background(), 20*time.Second)
	defer cancel()
	after, err := captureMessagesUnread(ctx, messagesUnreadPath())
	passed := err == nil && before.matches(after)
	result.Assertions = append(result.Assertions, Assertion{Name: "prelaunch_unread_baseline_restored", Passed: passed})
	if result.Metrics == nil {
		result.Metrics = map[string]any{}
	}
	result.Metrics["unread_baseline"] = map[string]any{"before_unread_chat_count": len(before), "after_unread_chat_count": len(after), "passed": passed}
	if !passed {
		result.Passed = false
		result.Status = "environment-limited"
	}
}
