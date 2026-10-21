package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func failedSelfFixture(ctx context.Context, database *sql.DB, own []string) (string, string, error) {
	run := os.Getenv("APPLE_BRIDGE_RESEND_RUN")
	if !regexp.MustCompile(`^[A-Za-z0-9_-]+$`).MatchString(run) {
		return "", "", contracts.ErrInvalid
	}
	data, err := os.ReadFile(filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-command-cleanup", run+".json"))
	var manifest struct {
		RunID    string `json:"run_id"`
		Baseline int64  `json:"baseline_max_row_id"`
	}
	if err != nil || json.Unmarshal(data, &manifest) != nil || manifest.RunID != run || manifest.Baseline <= 0 {
		return "", "", contracts.ErrInvalid
	}
	tag := "[ab-test " + run + "]"
	rows, err := database.QueryContext(ctx, "SELECT m.guid,c.guid FROM message m JOIN chat_message_join j ON j.message_id=m.ROWID JOIN chat c ON c.ROWID=j.chat_id WHERE m.ROWID>? AND m.is_from_me=1 AND m.error<>0 AND c.service_name='iMessage' AND (instr(m.text,?)>0 OR instr(m.attributedBody,CAST(? AS BLOB))>0) AND NOT EXISTS(SELECT 1 FROM chat_recoverable_message_join r WHERE r.message_id=m.ROWID AND r.chat_id=c.ROWID)", manifest.Baseline, tag, tag)
	if err != nil {
		return "", "", err
	}
	var native, chat string
	count := 0
	for rows.Next() {
		if err = rows.Scan(&native, &chat); err != nil {
			rows.Close()
			return "", "", err
		}
		count++
	}
	err = rows.Err()
	rows.Close()
	if err != nil || count != 1 {
		return "", "", contracts.ErrNotFound
	}
	peers, err := database.QueryContext(ctx, "SELECT h.id FROM chat c JOIN chat_handle_join j ON j.chat_id=c.ROWID JOIN handle h ON h.ROWID=j.handle_id WHERE c.guid=?", chat)
	if err != nil {
		return "", "", err
	}
	defer peers.Close()
	count = 0
	for peers.Next() {
		var handle string
		if peers.Scan(&handle) != nil {
			return "", "", contracts.ErrCorrupt
		}
		matched := false
		for _, candidate := range own {
			matched = matched || contracts.NormalizeContactHandle(handle) == contracts.NormalizeContactHandle(candidate)
		}
		if !matched {
			return "", "", contracts.ErrUnauthorized
		}
		count++
	}
	if peers.Err() != nil || count != 1 {
		return "", "", contracts.ErrUnsupported
	}
	return native, chat, nil
}
