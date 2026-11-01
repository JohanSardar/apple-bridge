package scenarios

import (
	"context"
	"database/sql"
	"testing"
)

func TestNativeMessagesSelfTestCleanupTruth(t *testing.T) {
	database, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer database.Close()
	for _, query := range []string{
		`CREATE TABLE message(text TEXT,attributedBody BLOB)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE attachment(filename TEXT)`,
		`CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER)`,
		`INSERT INTO message VALUES('[ab-test owned] text',NULL),(NULL,CAST('[ab-test owned] body' AS BLOB)),(NULL,NULL),('[ab-test owned] deleted',NULL),('[ab-test previous] text',NULL),('existing message',NULL)`,
		`INSERT INTO chat_message_join VALUES(1,1),(1,1),(1,2),(1,3),(1,4),(1,5),(1,6)`,
		`INSERT INTO chat_recoverable_message_join VALUES(1,4)`,
		`INSERT INTO attachment VALUES('/synthetic/ab-owned.caf'),('/synthetic/ab-previous.caf')`,
		`INSERT INTO message_attachment_join VALUES(3,1),(3,1),(5,2)`,
	} {
		if _, err := database.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	ctx := context.Background()
	texts, files, err := nativeMessagesSelfTestRemaining(ctx, database, "owned")
	if err != nil || texts != 2 || files != 1 {
		t.Fatal(texts, files, err)
	}
	if _, err := database.Exec(`INSERT INTO chat_recoverable_message_join VALUES(1,1),(1,2),(1,3)`); err != nil {
		t.Fatal(err)
	}
	texts, files, err = nativeMessagesSelfTestRemaining(ctx, database, "owned")
	if err != nil || texts != 0 || files != 0 {
		t.Fatal(texts, files, err)
	}
	var count int
	if err := database.QueryRow(`SELECT COUNT(*) FROM message`).Scan(&count); err != nil || count != 6 {
		t.Fatal("truth query changed native rows", count, err)
	}
	if _, err := database.Exec(`DROP TABLE attachment`); err != nil {
		t.Fatal(err)
	}
	if _, _, err := nativeMessagesSelfTestRemaining(ctx, database, "owned"); err == nil {
		t.Fatal("unavailable attachment truth reported as clean")
	}
}
