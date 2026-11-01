package scenarios

import (
	"context"
	"database/sql"
	"path/filepath"
	"testing"
)

func TestMessagesUnreadAuditDetectsActivationBeforeSnapshot(t *testing.T) {
	before := messageUnreadBaseline{"self": {unread: map[string]bool{"existing-a": true, "existing-b": true}, count: 2}}
	activated := messageUnreadBaseline{}
	if before.matches(activated) {
		t.Fatal("opening Messages must not erase the unread baseline before the helper captures it")
	}
	if !before.matches(before) {
		t.Fatal("an unchanged unread baseline must pass")
	}
}

func TestMessagesUnreadAuditAcceptsOwnerRestoreCriteria(t *testing.T) {
	before := messageUnreadBaseline{"self": {unread: map[string]bool{"existing": true}, count: 2}}
	for name, restored := range map[string]messageUnreadBaseline{
		"same identities": {"self": {unread: map[string]bool{"existing": true}, count: 1}},
		"same count":      {"self": {unread: map[string]bool{"replacement": true}, count: 2}},
	} {
		if !before.matches(restored) {
			t.Errorf("%s must satisfy the owner's restore decision", name)
		}
	}
}

func TestMessagesUnreadAuditProtectsOtherChats(t *testing.T) {
	before := messageUnreadBaseline{"other": {unread: map[string]bool{"existing": true}, count: 1}}
	if before.matches(messageUnreadBaseline{}) {
		t.Fatal("activating a previously selected other chat must fail the independent audit")
	}
	if (messageUnreadBaseline{}).matches(before) {
		t.Fatal("new unread state in another chat must fail the independent audit")
	}
}

func TestMessagesUnreadAuditDatabaseRoundTrip(t *testing.T) {
	path := filepath.Join(t.TempDir(), "chat.db")
	db, err := sql.Open("sqlite", path)
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	_, err = db.Exec(`
CREATE TABLE chat (ROWID INTEGER PRIMARY KEY, guid TEXT, last_read_message_timestamp INTEGER);
CREATE TABLE message (ROWID INTEGER PRIMARY KEY, guid TEXT, is_from_me INTEGER, is_read INTEGER, date INTEGER);
CREATE TABLE chat_message_join (chat_id INTEGER, message_id INTEGER);
CREATE TABLE chat_recoverable_message_join (chat_id INTEGER, message_id INTEGER);
INSERT INTO chat VALUES (1,'self',5),(2,'other',5);
INSERT INTO message VALUES (1,'self-unread',0,0,10),(2,'other-unread',0,0,3),(3,'read',0,1,4),(4,'outgoing',1,0,10),(5,'deleted',0,0,10);
INSERT INTO chat_message_join VALUES (1,1),(2,2),(1,3),(1,4),(1,5);
INSERT INTO chat_recoverable_message_join VALUES (1,5);
`)
	if err != nil {
		t.Fatal(err)
	}
	capture := func() messageUnreadBaseline {
		t.Helper()
		state, err := captureMessagesUnread(context.Background(), path)
		if err != nil {
			t.Fatal(err)
		}
		return state
	}
	before := capture()
	if len(before) != 2 || len(before["self"].unread) != 1 || !before["self"].unread["self-unread"] || before["self"].count != 1 || before["other"].count != 0 {
		t.Fatal("the snapshot must include incoming active unread state and exclude outgoing and deleted rows")
	}
	if _, err := db.Exec("UPDATE message SET is_read=1 WHERE ROWID=1; UPDATE chat SET last_read_message_timestamp=20 WHERE ROWID=1"); err != nil {
		t.Fatal(err)
	}
	if before.matches(capture()) {
		t.Fatal("the prelaunch snapshot must detect Messages reading the existing self conversation")
	}
	if _, err := db.Exec("UPDATE chat SET last_read_message_timestamp=5 WHERE ROWID=1"); err != nil {
		t.Fatal(err)
	}
	if !before.matches(capture()) {
		t.Fatal("restoring the conversation unread count must satisfy the owner's restore criterion")
	}
}
