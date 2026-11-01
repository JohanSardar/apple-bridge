package scenarios

import (
	"context"
	"database/sql"
	"os"
	"regexp"
	"strings"
	"testing"
)

func TestMessagesWritePreflightPreservesExistingUnreadMessages(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(is_from_me INTEGER,is_read INTEGER,associated_message_type INTEGER DEFAULT 0)`,
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
	for _, id := range []string{"MSG-READSTATE-REAL", "MSG-DELETE-REAL", "MSG-RESEND", "WEB-MEDIA-VARIANTS", "WEB-UPLOAD-NATIVE-STAGING"} {
		e, allowed := nativeMessagesExistingChatGate(ctx, db, id, "self", Evidence{Passed: true})
		if allowed || e.Passed || e.Metrics["writes_submitted"] != 0 || e.Conditions["reason"] == "" {
			t.Fatal(id, e, allowed)
		}
	}
	for _, id := range []string{"WEB-MSG-REAL-READ"} {
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

func TestMessagesWritePreflightIgnoresUnreadTapbackEvent(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(is_from_me INTEGER,is_read INTEGER,associated_message_type INTEGER)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`INSERT INTO chat(guid) VALUES('self')`,
		`INSERT INTO message VALUES(0,0,3006)`,
		`INSERT INTO chat_message_join VALUES(1,1)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	for _, event := range []int{1000, 2000, 3006} {
		if _, err := db.Exec(`UPDATE message SET associated_message_type=?`, event); err != nil {
			t.Fatal(err)
		}
		for _, id := range []string{"MSG-READSTATE-REAL", "MSG-DELETE-REAL", "MSG-RESEND", "WEB-MEDIA-VARIANTS", "WEB-UPLOAD-NATIVE-STAGING"} {
			if _, allowed := nativeMessagesExistingChatGate(context.Background(), db, id, "self", Evidence{}); !allowed {
				t.Fatal(id, "reaction event treated as an unread message", event)
			}
		}
	}
	if _, err := db.Exec(`UPDATE message SET associated_message_type=0`); err != nil {
		t.Fatal(err)
	}
	if _, allowed := nativeMessagesExistingChatGate(context.Background(), db, "MSG-DELETE-REAL", "self", Evidence{}); allowed {
		t.Fatal("genuine unread message allowed")
	}
}

func TestNativeUnreadPreflightQueriesPreserveGenuineUnreadMessages(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(is_from_me INTEGER,is_read INTEGER,associated_message_type INTEGER)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`INSERT INTO chat(guid) VALUES('self')`,
		`INSERT INTO message VALUES(0,1,0),(0,0,3006)`,
		`INSERT INTO chat_message_join VALUES(1,1),(1,2)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	pattern := regexp.MustCompile(`"(SELECT 1[^"\n]*m\.is_read=0[^"\n]*)"`)
	count := 0
	for _, name := range []string{"MessagesSelfTestBaseline.swift", "MessagesReadStateFixture.swift", "MessagesResendFixture.swift"} {
		source, err := os.ReadFile("../../native/Sources/MessagesSource/" + name)
		if err != nil {
			t.Fatal(err)
		}
		for _, match := range pattern.FindAllStringSubmatch(string(source), -1) {
			count++
			args := []any{"self"}
			if strings.Count(match[1], "?") == 2 {
				args = append(args, 10)
			}
			for _, unread := range []bool{false, true} {
				read := 1
				if unread {
					read = 0
				}
				if _, err := db.Exec("UPDATE message SET is_read=? WHERE ROWID=1", read); err != nil {
					t.Fatal(err)
				}
				rows, err := db.Query(match[1], args...)
				if err != nil {
					t.Fatal(name, err)
				}
				found := rows.Next()
				problem := rows.Err()
				rows.Close()
				if problem != nil || found != unread {
					t.Fatal(name, "unread", unread, "found", found, problem)
				}
			}
		}
	}
	if count != 4 {
		t.Fatal("native unread query coverage changed", count)
	}
}
