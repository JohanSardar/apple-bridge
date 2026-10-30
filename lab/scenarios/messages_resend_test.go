package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestFailedSelfFixtureIsReadOnlyAndScoped(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("APPLE_BRIDGE_RESEND_RUN", "owned")
	directory := filepath.Join(home, "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup")
	if err := os.MkdirAll(directory, 0700); err != nil {
		t.Fatal(err)
	}
	manifest, _ := json.Marshal(map[string]any{"run_id": "owned", "baseline_max_row_id": 10})
	if err := os.WriteFile(filepath.Join(directory, "owned.json"), manifest, 0600); err != nil {
		t.Fatal(err)
	}
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	db.SetMaxOpenConns(1)
	_, err = db.Exec(`CREATE TABLE message(guid TEXT,is_from_me INTEGER,error INTEGER,text TEXT,attributedBody BLOB);
 CREATE TABLE chat(guid TEXT,service_name TEXT);
 CREATE TABLE chat_message_join(chat_id INTEGER,message_id INTEGER);
 CREATE TABLE chat_handle_join(chat_id INTEGER,handle_id INTEGER);
 CREATE TABLE handle(id TEXT);
 CREATE TABLE chat_recoverable_message_join(chat_id INTEGER,message_id INTEGER);
 INSERT INTO handle VALUES('self@example.com');
 INSERT INTO chat VALUES('self','iMessage');
 INSERT INTO chat_handle_join VALUES(1,1);
 INSERT INTO message(ROWID,guid,is_from_me,error,text) VALUES(9,'residue',1,1,'[ab-test old]'),(11,'owned-failed',1,1,'[ab-test owned]');
 INSERT INTO chat_message_join VALUES(1,9),(1,11);`)
	if err != nil {
		t.Fatal(err)
	}
	ctx := context.Background()
	native, chat, err := failedSelfFixture(ctx, db, []string{"self@example.com"})
	if err != nil || native != "owned-failed" || chat != "self" {
		t.Fatal(native, chat, err)
	}
	if _, _, err = failedSelfFixture(ctx, db, []string{"other@example.com"}); err == nil {
		t.Fatal("foreign self route accepted")
	}
	if _, err = db.Exec("INSERT INTO chat_recoverable_message_join VALUES(1,11)"); err != nil {
		t.Fatal(err)
	}
	if _, _, err = failedSelfFixture(ctx, db, []string{"self@example.com"}); err == nil {
		t.Fatal("deleted fixture accepted")
	}
	var count int
	if err = db.QueryRow("SELECT COUNT(*) FROM message").Scan(&count); err != nil || count != 2 {
		t.Fatal("fixture lookup changed residue", count, err)
	}
}
