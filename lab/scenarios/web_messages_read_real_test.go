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
