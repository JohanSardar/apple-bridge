package scenarios

import (
	"context"
	"database/sql"
	"testing"
)

func TestReadStateFixtureRejectsExistingOrMixedConversations(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(guid TEXT,text TEXT,attributedBody BLOB)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER)`,
		`INSERT INTO chat(ROWID,guid) VALUES(11,'self')`,
		`INSERT INTO message(guid,text) VALUES('owned','[ab-test current] fixture')`,
		`INSERT INTO chat_message_join VALUES(11,1)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	ctx := context.Background()
	if !isolatedReadStateFixture(ctx, db, "owned", "[ab-test current] fixture", 10) {
		t.Fatal("new owned conversation refused")
	}
	if isolatedReadStateFixture(ctx, db, "owned", "[ab-test current] fixture", 11) {
		t.Fatal("existing conversation allowed")
	}
	if _, err := db.Exec(`INSERT INTO message(guid,text) VALUES('foreign','personal'); INSERT INTO chat_message_join VALUES(11,2)`); err != nil {
		t.Fatal(err)
	}
	if isolatedReadStateFixture(ctx, db, "owned", "[ab-test current] fixture", 10) {
		t.Fatal("mixed conversation allowed")
	}
}
