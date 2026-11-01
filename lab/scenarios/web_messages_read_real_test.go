package scenarios

import (
	"context"
	"database/sql"
	"testing"
)

func TestNativeMessagesReadWaitsForCommittedRecord(t *testing.T) {
	database, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	for _, query := range []string{
		`CREATE TABLE aliases(collection TEXT,native_key TEXT,record_id TEXT)`,
		`CREATE TABLE records(collection TEXT,id TEXT,deleted INTEGER)`,
		`INSERT INTO aliases VALUES('messages.message','native','message')`,
	} {
		if _, err := database.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	ctx := context.Background()
	if _, captured := capturedNativeMessagesRecord(ctx, database, "messages.message", "native"); captured {
		t.Fatal("an alias allocated by an attachment reference is not a captured message")
	}
	if _, err := database.Exec(`INSERT INTO records VALUES('messages.message','message',1)`); err != nil {
		t.Fatal(err)
	}
	if _, captured := capturedNativeMessagesRecord(ctx, database, "messages.message", "native"); captured {
		t.Fatal("a deleted record qualified as captured")
	}
	if _, err := database.Exec(`UPDATE records SET deleted=0`); err != nil {
		t.Fatal(err)
	}
	if id, captured := capturedNativeMessagesRecord(ctx, database, "messages.message", "native"); !captured || id != "message" {
		t.Fatal(id, captured)
	}
	if _, captured := capturedNativeMessagesRecord(ctx, database, "messages.chat", "native"); captured {
		t.Fatal("record identity crossed collections")
	}
}

func TestNativeWebReadSelectsVisibleTextInsteadOfAttachmentPlaceholder(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(guid TEXT,text TEXT,is_from_me INTEGER,item_type INTEGER,associated_message_type INTEGER,date_retracted INTEGER)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE handle(id TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER)`,
		`INSERT INTO chat VALUES('self')`,
		`INSERT INTO handle VALUES('self@example.com')`,
		`INSERT INTO chat_handle_join VALUES(1,1)`,
		`INSERT INTO message VALUES('text','[ab-test] visible text',1,0,0,0),('file',char(65532),0,0,0,0),('reaction','removed',0,0,3006,0),('unsent','',1,0,0,0)`,
		`INSERT INTO chat_message_join VALUES(1,1),(1,2),(1,3),(1,4)`,
		`INSERT INTO message_attachment_join VALUES(2,1)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	var guid, text, peer string
	var outgoing bool
	if err := db.QueryRow(nativeWebSelfTextQuery, "self").Scan(&guid, &text, &outgoing, &peer); err != nil {
		t.Fatal(err)
	}
	if guid != "text" || text != "[ab-test] visible text" || !outgoing || peer != "self@example.com" {
		t.Fatal("did not select independent plain-text truth")
	}
}
