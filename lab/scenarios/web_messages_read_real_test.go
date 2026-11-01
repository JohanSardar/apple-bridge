package scenarios

import (
	"database/sql"
	"testing"
)

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
