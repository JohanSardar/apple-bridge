package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) initReadModels(ctx context.Context) error {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `CREATE INDEX IF NOT EXISTS reminder_list ON records(json_extract(payload,'$.data.list_id'),id) WHERE collection='reminders.reminder' AND deleted=0;
 CREATE INDEX IF NOT EXISTS reminder_attachment ON records(json_extract(payload,'$.data.reminder_id'),id) WHERE collection='reminders.attachment' AND deleted=0;
 CREATE TABLE IF NOT EXISTS msg_by_chat(id TEXT PRIMARY KEY,chat_id TEXT NOT NULL,sent_at TEXT NOT NULL,unread INTEGER NOT NULL,originator TEXT NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS msg_chat_time ON msg_by_chat(chat_id,sent_at,id);
 CREATE INDEX IF NOT EXISTS msg_unread ON msg_by_chat(chat_id,sent_at,id) WHERE unread=1;
 CREATE INDEX IF NOT EXISTS msg_mention ON msg_by_chat(chat_id) WHERE json_extract(payload,'$.data.has_unseen_mention')=1 AND json_extract(payload,'$.data.deleted_at') IS NULL;
 CREATE INDEX IF NOT EXISTS msg_deleted ON msg_by_chat(chat_id) WHERE json_extract(payload,'$.data.deleted_at') IS NOT NULL;
 CREATE INDEX IF NOT EXISTS msg_spam ON msg_by_chat(chat_id) WHERE json_extract(payload,'$.data.is_spam')=1;
 CREATE INDEX IF NOT EXISTS msg_balloon ON msg_by_chat(chat_id,json_extract(payload,'$.data.balloon.kind'),sent_at DESC,id);
 CREATE INDEX IF NOT EXISTS msg_originator ON msg_by_chat(originator);
 CREATE TABLE IF NOT EXISTS reaction_by_message(id TEXT PRIMARY KEY,message_id TEXT NOT NULL,at TEXT NOT NULL,seq INTEGER NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS reaction_message ON reaction_by_message(message_id,at,seq,id);
 CREATE TABLE IF NOT EXISTS attachment_by_message(id TEXT PRIMARY KEY,message_id TEXT NOT NULL,kind TEXT NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS attachment_message ON attachment_by_message(message_id,kind,id);
 CREATE TABLE IF NOT EXISTS activity_by_chat(id TEXT PRIMARY KEY,chat_id TEXT NOT NULL,at TEXT NOT NULL,message_id TEXT NOT NULL);
 CREATE INDEX IF NOT EXISTS activity_chat ON activity_by_chat(chat_id,at DESC,id DESC);
 CREATE INDEX IF NOT EXISTS activity_message ON activity_by_chat(message_id);
 CREATE TABLE IF NOT EXISTS chat_summary(id TEXT PRIMARY KEY,last_activity_at TEXT NOT NULL DEFAULT '',unread_count INTEGER NOT NULL DEFAULT 0);
 CREATE INDEX IF NOT EXISTS chat_activity ON chat_summary(last_activity_at DESC,id);
 CREATE TRIGGER IF NOT EXISTS rm_msg_insert AFTER INSERT ON msg_by_chat BEGIN
 INSERT INTO chat_summary(id,unread_count) VALUES(new.chat_id,new.unread) ON CONFLICT(id) DO UPDATE SET unread_count=unread_count+new.unread;
 END;
 CREATE TRIGGER IF NOT EXISTS rm_msg_delete AFTER DELETE ON msg_by_chat BEGIN
 UPDATE chat_summary SET unread_count=unread_count-old.unread WHERE id=old.chat_id;
 END;
 CREATE TRIGGER IF NOT EXISTS rm_activity_insert AFTER INSERT ON activity_by_chat BEGIN
 INSERT INTO chat_summary(id,last_activity_at) VALUES(new.chat_id,new.at) ON CONFLICT(id) DO UPDATE SET last_activity_at=coalesce((SELECT at FROM activity_by_chat WHERE chat_id=new.chat_id ORDER BY at DESC,id DESC LIMIT 1),'');
 END;
 CREATE TRIGGER IF NOT EXISTS rm_activity_delete AFTER DELETE ON activity_by_chat BEGIN
 UPDATE chat_summary SET last_activity_at=coalesce((SELECT at FROM activity_by_chat WHERE chat_id=old.chat_id ORDER BY at DESC,id DESC LIMIT 1),'') WHERE id=old.chat_id;
 END;`)
	if err != nil {
		return err
	}
	if err = initChatParticipantsTx(ctx, tx); err != nil {
		return err
	}
	var version string
	err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='readmodels_version'").Scan(&version)
	if err != nil && err != sql.ErrNoRows {
		return err
	}
	if version == "3" {
		return tx.Commit()
	}
	rows, err := tx.QueryContext(ctx, "SELECT payload,seq FROM records WHERE collection LIKE 'messages.%' ORDER BY CASE collection WHEN 'messages.message' THEN 0 ELSE 1 END,id")
	if err != nil {
		return err
	}
	type entry struct {
		record contracts.Record
		seq    uint64
	}
	entries := []entry{}
	for rows.Next() {
		var raw []byte
		var e entry
		if err = rows.Scan(&raw, &e.seq); err != nil {
			break
		}
		if err = json.Unmarshal(raw, &e.record); err != nil {
			break
		}
		entries = append(entries, e)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		return err
	}
	for _, e := range entries {
		if err = writeReadModel(ctx, tx, e.record, e.seq); err != nil {
			return err
		}
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO meta(key,value) VALUES('readmodels_version','3') ON CONFLICT(key) DO UPDATE SET value='3'"); err != nil {
		return err
	}
	return tx.Commit()
}
func modelFields(r contracts.Record) map[string]any {
	var fields map[string]any
	json.Unmarshal(r.Data, &fields)
	return fields
}
func fieldString(fields map[string]any, key string) string { v, _ := fields[key].(string); return v }
func fieldBool(fields map[string]any, key string) bool     { v, _ := fields[key].(bool); return v }
func attachmentKind(d map[string]any) string {
	mime := strings.ToLower(fieldString(d, "mime_type"))
	switch {
	case strings.HasPrefix(mime, "image/"):
		if fieldBool(d, "is_sticker") {
			return "stickers"
		}
		return "photos"
	case strings.HasPrefix(mime, "video/"):
		return "videos"
	case strings.HasPrefix(mime, "audio/"):
		return "audio"
	default:
		return "documents"
	}
}
func writeReadModel(ctx context.Context, tx *sql.Tx, r contracts.Record, seq uint64) error {
	switch r.Collection {
	case "messages.handle", "messages.chat", "messages.message", "messages.reaction", "messages.attachment":
	default:
		return nil
	}
	d := modelFields(r)
	payload, err := json.Marshal(r)
	if err != nil {
		return err
	}
	switch r.Collection {
	case "messages.handle":
		return writeChatParticipantsTx(ctx, tx, r, d)
	case "messages.chat":
		if err = writeChatParticipantsTx(ctx, tx, r, d); err != nil {
			return err
		}
		_, err = tx.ExecContext(ctx, "INSERT OR IGNORE INTO chat_summary(id) VALUES(?)", r.ID)
	case "messages.message":
		if _, err = tx.ExecContext(ctx, "DELETE FROM msg_by_chat WHERE id=?", r.ID); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, "DELETE FROM activity_by_chat WHERE message_id=?", r.ID); err != nil {
			return err
		}
		if r.Deleted {
			return nil
		}
		unread := 0
		if !fieldBool(d, "is_from_me") && fieldString(d, "kind") == "message" && d["read_at"] == nil && d["deleted_at"] == nil {
			unread = 1
		}
		chat := fieldString(d, "chat_id")
		at := modelTime(fieldString(d, "sent_at"))
		_, err = tx.ExecContext(ctx, "INSERT INTO msg_by_chat VALUES(?,?,?,?,?,?)", r.ID, chat, at, unread, fieldString(d, "thread_originator_id"), payload)
		if err == nil {
			_, err = tx.ExecContext(ctx, "INSERT INTO activity_by_chat VALUES(?,?,?,?)", r.ID, chat, at, r.ID)
		}
		if err == nil {
			_, err = tx.ExecContext(ctx, "INSERT INTO activity_by_chat SELECT id,?,at,message_id FROM reaction_by_message WHERE message_id=?", chat, r.ID)
		}
	case "messages.reaction":
		if _, err = tx.ExecContext(ctx, "DELETE FROM reaction_by_message WHERE id=?", r.ID); err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, "DELETE FROM activity_by_chat WHERE id=?", r.ID); err != nil {
			return err
		}
		if r.Deleted {
			return nil
		}
		message := fieldString(d, "message_id")
		at := modelTime(fieldString(d, "sent_at"))
		if at == "" {
			at = r.ObservedAt.UTC().Format("2006-01-02T15:04:05.000000000Z")
		}
		_, err = tx.ExecContext(ctx, "INSERT INTO reaction_by_message VALUES(?,?,?,?,?)", r.ID, message, at, seq, payload)
		if err == nil {
			_, err = tx.ExecContext(ctx, "INSERT INTO activity_by_chat SELECT ?,chat_id,?,id FROM msg_by_chat WHERE id=?", r.ID, at, message)
		}
	case "messages.attachment":
		if _, err = tx.ExecContext(ctx, "DELETE FROM attachment_by_message WHERE id=?", r.ID); err != nil {
			return err
		}
		if !r.Deleted {
			messageID := attachmentMessage(d)
			_, err = tx.ExecContext(ctx, "INSERT INTO attachment_by_message VALUES(?,?,?,?)", r.ID, messageID, attachmentKind(d), payload)
		}
	}
	return err
}

func attachmentMessage(d map[string]any) string {
	if sticker, ok := d["sticker"].(map[string]any); ok && fieldString(sticker, "target_message_id") != "" {
		return fieldString(sticker, "target_message_id")
	}
	return fieldString(d, "message_id")
}
