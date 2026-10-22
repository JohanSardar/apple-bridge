package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func initChatParticipantsTx(ctx context.Context, tx *sql.Tx) error {
	if _, err := tx.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS handle_address(id TEXT PRIMARY KEY,address TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS handle_address_value ON handle_address(address,id);
 CREATE TABLE IF NOT EXISTS chat_participant(chat_id TEXT NOT NULL,handle_id TEXT NOT NULL,PRIMARY KEY(chat_id,handle_id));
 CREATE INDEX IF NOT EXISTS chat_participant_handle ON chat_participant(handle_id,chat_id);`); err != nil {
		return err
	}
	var version string
	err := tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='chat_participants_version'").Scan(&version)
	if err != nil && err != sql.ErrNoRows {
		return err
	}
	if version == "1" {
		return nil
	}
	records, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection IN ('messages.handle','messages.chat') ORDER BY collection,id")
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM handle_address; DELETE FROM chat_participant;"); err != nil {
		return err
	}
	for _, record := range records {
		if err = writeChatParticipantsTx(ctx, tx, record, modelFields(record)); err != nil {
			return err
		}
	}
	_, err = tx.ExecContext(ctx, "INSERT INTO meta(key,value) VALUES('chat_participants_version','1') ON CONFLICT(key) DO UPDATE SET value='1'")
	return err
}

func writeChatParticipantsTx(ctx context.Context, tx *sql.Tx, record contracts.Record, data map[string]any) error {
	if record.Collection == "messages.handle" {
		if _, err := tx.ExecContext(ctx, "DELETE FROM handle_address WHERE id=?", record.ID); err != nil {
			return err
		}
		address := contracts.NormalizeContactHandle(fieldString(data, "address"))
		if record.Deleted || address == "" {
			return nil
		}
		_, err := tx.ExecContext(ctx, "INSERT INTO handle_address(id,address) VALUES(?,?)", record.ID, address)
		return err
	}
	if _, err := tx.ExecContext(ctx, "DELETE FROM chat_participant WHERE chat_id=?", record.ID); err != nil {
		return err
	}
	if record.Deleted {
		return nil
	}
	participants, _ := data["participants"].([]any)
	for _, raw := range participants {
		if id, ok := raw.(string); ok && id != "" {
			if _, err := tx.ExecContext(ctx, "INSERT OR IGNORE INTO chat_participant(chat_id,handle_id) VALUES(?,?)", record.ID, id); err != nil {
				return err
			}
		}
	}
	return nil
}

func exactParticipantPredicate(handles []string) (string, []any) {
	slots := strings.TrimSuffix(strings.Repeat("?,", len(handles)), ",")
	query := "r.id IN (SELECT p.chat_id FROM handle_address h INDEXED BY handle_address_value JOIN chat_participant p INDEXED BY chat_participant_handle ON p.handle_id=h.id WHERE h.address IN (" + slots + ") GROUP BY p.chat_id HAVING COUNT(DISTINCT h.address)=?) AND NOT EXISTS(SELECT 1 FROM chat_participant extra LEFT JOIN handle_address address ON address.id=extra.handle_id WHERE extra.chat_id=r.id AND (address.address IS NULL OR address.address NOT IN (" + slots + ")))"
	args := []any{}
	for _, handle := range handles {
		args = append(args, handle)
	}
	args = append(args, len(handles))
	for _, handle := range handles {
		args = append(args, handle)
	}
	return query, args
}

func participantScope(filter string, handles []string) string {
	if handles == nil {
		return filter
	}
	value, _ := json.Marshal(handles)
	return filter + ":participants:" + string(value)
}
