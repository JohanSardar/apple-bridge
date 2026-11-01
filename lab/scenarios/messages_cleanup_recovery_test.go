package scenarios

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestCleanupRecoveryRejectsUnboundOrForeignLedgersBeforeNativeAccess(t *testing.T) {
	for _, scenario := range []string{"MSG-AUDIO-MESSAGE", "MSG-SELFTEST"} {
		t.Run(scenario, func(t *testing.T) {
			t.Setenv("HOME", t.TempDir())
			run := "variant_" + strings.Repeat("A", 32)
			t.Setenv("APPLE_BRIDGE_MESSAGE_CLEANUP_RUN", run)
			t.Setenv("APPLE_BRIDGE_MESSAGE_CLEANUP_CACHE", strings.Repeat("a", 64))
			dir := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup")
			if err := os.MkdirAll(dir, 0700); err != nil {
				t.Fatal(err)
			}
			data := []byte(`{"run_id":"` + run + `","scenario":"` + scenario + `","baseline_max_row_id":10}`)
			if err := os.WriteFile(filepath.Join(dir, run+".json"), data, 0600); err != nil {
				t.Fatal(err)
			}
			digest := sha256.Sum256(data)
			if scenario == "MSG-SELFTEST" {
				t.Setenv("APPLE_BRIDGE_MESSAGE_CLEANUP_LEDGER_SHA256", hex.EncodeToString(digest[:]))
			}
			e := runMessagesCleanupRecovery(context.Background(), Evidence{})
			if e.Passed || e.Metrics["writes_submitted"] != 0 || len(e.Assertions) != 1 || e.Assertions[0].Name != "unchanged_current_run_ledger" || e.Assertions[0].Passed {
				t.Fatal(e)
			}
		})
	}
}

func TestCleanupRecoveryRecognizesCurrentTextAttachmentEchoOnly(t *testing.T) {
	db, err := sql.Open("sqlite", ":memory:")
	if err != nil {
		t.Fatal(err)
	}
	defer db.Close()
	for _, query := range []string{
		`CREATE TABLE message(guid TEXT,text TEXT,attributedBody BLOB)`,
		`CREATE TABLE message_attachment_join(message_id INTEGER,attachment_id INTEGER)`,
		`CREATE TABLE attachment(filename TEXT,transfer_name TEXT)`,
		`INSERT INTO message(ROWID,guid) VALUES(1,'baseline'),(2,'txt-echo'),(3,'foreign')`,
		`INSERT INTO attachment(ROWID,filename,transfer_name) VALUES(1,'/tmp/ab-selftest_CURRENT.txt','ab-selftest_CURRENT.txt'),(2,'/tmp/ab-selftest_OTHER.txt','ab-selftest_OTHER.txt')`,
		`INSERT INTO message_attachment_join VALUES(1,1),(2,1),(3,2)`,
	} {
		if _, err := db.Exec(query); err != nil {
			t.Fatal(err)
		}
	}
	rows, err := nativeMessagesRecoveryOwnedRows(context.Background(), db, 1, "selftest_CURRENT")
	if err != nil {
		t.Fatal(err)
	}
	defer rows.Close()
	var ids []string
	for rows.Next() {
		var id string
		if err := rows.Scan(&id); err != nil {
			t.Fatal(err)
		}
		ids = append(ids, id)
	}
	if err := rows.Err(); err != nil {
		t.Fatal(err)
	}
	if len(ids) != 1 || ids[0] != "txt-echo" {
		t.Fatal(ids)
	}
}
