package scenarios

import (
	"context"
	"database/sql"
	"testing"
)

func TestMessagesWritePreflightPreservesExistingUnreadMessages(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(is_from_me INTEGER,is_read INTEGER)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`INSERT INTO chat(guid) VALUES('self')`,
		`INSERT INTO message(is_from_me,is_read) VALUES(0,0)`,
		`INSERT INTO chat_message_join VALUES(1,1)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	ctx := context.Background()
	for _, id := range []string{"MSG-DELETE-REAL", "WEB-MEDIA-VARIANTS", "WEB-UPLOAD-NATIVE-STAGING"} {
		e, allowed := nativeMessagesExistingChatGate(ctx, db, id, "self", Evidence{Passed: true})
		if allowed || e.Passed || e.Metrics["writes_submitted"] != 0 || e.Conditions["reason"] == "" {
			t.Fatal(id, e, allowed)
		}
	}
	for _, id := range []string{"WEB-MSG-REAL-READ", "MSG-READSTATE-REAL"} {
		if _, allowed := nativeMessagesExistingChatGate(ctx, db, id, "self", Evidence{}); !allowed {
			t.Fatal("non-mutating or fresh-conversation check refused", id)
		}
	}
	if _, err := db.Exec(`UPDATE message SET is_read=1`); err != nil {
		t.Fatal(err)
	}
	if _, allowed := nativeMessagesExistingChatGate(ctx, db, "WEB-MEDIA-VARIANTS", "self", Evidence{}); !allowed {
		t.Fatal("already-read self-chat refused")
	}
	if _, err := db.Exec(`UPDATE message SET is_read=0; INSERT INTO chat_recoverable_message_join VALUES(1,1)`); err != nil {
		t.Fatal(err)
	}
	if _, allowed := nativeMessagesExistingChatGate(ctx, db, "WEB-MEDIA-VARIANTS", "self", Evidence{}); !allowed {
		t.Fatal("deleted unread history treated as active")
	}
}
