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

func TestReadStateBaselineDetectsChangesAndExcludesOwnedRows(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(guid TEXT,is_read INTEGER,date_read INTEGER,attributedBody BLOB)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date INTEGER)`,
		`INSERT INTO chat(guid) VALUES('self')`,
		`INSERT INTO message(guid,is_read,date_read,attributedBody) VALUES('existing',1,20,X'0011')`,
		`INSERT INTO chat_message_join VALUES(1,1)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	ctx := context.Background()
	before, err := nativeMessagesChatDigest(ctx, db, "self", 1)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`INSERT INTO message(guid,is_read) VALUES('owned',0); INSERT INTO chat_message_join VALUES(1,2)`); err != nil {
		t.Fatal(err)
	}
	after, err := nativeMessagesChatDigest(ctx, db, "self", 1)
	if err != nil || before != after {
		t.Fatal("owned row changed existing baseline", err)
	}
	for _, query := range []string{
		`UPDATE message SET is_read=0 WHERE ROWID=1`,
		`UPDATE message SET is_read=1,date_read=21 WHERE ROWID=1`,
		`UPDATE message SET date_read=20,attributedBody=X'0022' WHERE ROWID=1`,
		`DELETE FROM chat_message_join WHERE message_id=1`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
		after, err := nativeMessagesChatDigest(ctx, db, "self", 1)
		if err != nil || before == after {
			t.Fatal("existing row change went undetected", query, err)
		}
	}
}

func TestReadStateBaselineIgnoresReactionReadBookkeepingOnly(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(guid TEXT,is_read INTEGER,date_read INTEGER,associated_message_type INTEGER)`,
		`CREATE TABLE chat(guid TEXT)`,
		`CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER)`,
		`CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER,delete_date INTEGER)`,
		`INSERT INTO chat VALUES('self')`,
		`INSERT INTO message VALUES('message',1,10,0),('reaction',0,0,3006)`,
		`INSERT INTO chat_message_join VALUES(1,1),(1,2)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	before, err := nativeMessagesChatDigest(context.Background(), db, "self", 2)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := db.Exec(`UPDATE message SET is_read=1,date_read=20 WHERE guid='reaction'`); err != nil {
		t.Fatal(err)
	}
	after, err := nativeMessagesChatDigest(context.Background(), db, "self", 2)
	if err != nil || before != after {
		t.Fatal("reaction read bookkeeping changed message baseline", err)
	}
	if _, err := db.Exec(`UPDATE message SET associated_message_type=2006 WHERE guid='reaction'`); err != nil {
		t.Fatal(err)
	}
	after, err = nativeMessagesChatDigest(context.Background(), db, "self", 2)
	if err != nil || before == after {
		t.Fatal("reaction content change was ignored", err)
	}
}
