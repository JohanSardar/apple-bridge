package scenarios

import (
	"database/sql"
	"os"
	"regexp"
	"testing"
)

func TestSelfChatBaselineTargetsModernText(t *testing.T) {
	source, err := os.ReadFile("../../native/Sources/MessagesSource/MessagesSelfTestBaseline.swift")
	if err != nil {
		t.Fatal(err)
	}
	match := regexp.MustCompile(`"(SELECT c.service_name,m.guid FROM chat c JOIN chat_message_join[^"\n]+)"`).FindSubmatch(source)
	if len(match) != 2 {
		t.Fatal("missing baseline target query")
	}
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE chat(guid TEXT,service_name TEXT)`,
		`CREATE TABLE message(guid TEXT,is_from_me INTEGER,associated_message_type INTEGER,item_type INTEGER,date_retracted INTEGER,text TEXT,attributedBody BLOB)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE message_attachment_join(message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`INSERT INTO chat VALUES('self','iMessage')`,
		`INSERT INTO message VALUES('legacy',1,0,0,0,'old',NULL),('modern',1,0,0,0,NULL,X'74657874'),('reaction',1,3006,0,0,'reaction',NULL),('retracted',1,0,0,1,'retracted',NULL),('media',1,0,0,0,NULL,X'6D65646961'),('deleted',1,0,0,0,'deleted',NULL),('empty',1,0,0,0,NULL,NULL),('incoming',0,0,0,0,'incoming',NULL)`,
		`INSERT INTO chat_message_join SELECT 1,ROWID FROM message`,
		`INSERT INTO message_attachment_join VALUES(5)`,
		`INSERT INTO chat_recoverable_message_join VALUES(1,6)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	var service, guid string
	if err := db.QueryRow(string(match[1]), "self").Scan(&service, &guid); err != nil {
		t.Fatal(err)
	}
	if service != "iMessage" || guid != "modern" {
		t.Fatal(service, guid)
	}
}
