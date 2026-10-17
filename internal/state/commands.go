package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type CommandTransition struct {
	From   string
	Record contracts.CommandRecord
}

func (s *Store) InitCommands(ctx context.Context) error {
	_, err := s.db.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS commands(id TEXT PRIMARY KEY,principal TEXT NOT NULL,request_hash TEXT NOT NULL,state TEXT NOT NULL,payload BLOB NOT NULL);
 INSERT OR IGNORE INTO meta(key,value) VALUES('admission_era','`+contracts.NewID("")+`');`)
	return err
}
func (s *Store) AdmissionEra(ctx context.Context) (string, error) {
	var era string
	err := s.db.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='admission_era'").Scan(&era)
	return era, err
}
func (s *Store) Command(ctx context.Context, id string) (contracts.CommandRecord, error) {
	var r contracts.CommandRecord
	var b []byte
	err := s.db.QueryRowContext(ctx, "SELECT payload FROM commands WHERE id=?", id).Scan(&b)
	if errors.Is(err, sql.ErrNoRows) {
		return r, contracts.ErrNotFound
	}
	if err == nil {
		err = json.Unmarshal(b, &r)
	}
	return r, err
}
func (s *Store) Commands(ctx context.Context) ([]contracts.CommandRecord, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT payload FROM commands ORDER BY id")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []contracts.CommandRecord{}
	for rows.Next() {
		var b []byte
		var r contracts.CommandRecord
		if err = rows.Scan(&b); err != nil {
			return nil, err
		}
		if err = json.Unmarshal(b, &r); err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}
func commandTransitionAllowed(from, to string) bool {
	switch from {
	case "":
		return to == "admitted"
	case "admitted":
		return to == "prepared" || to == "rejected" || to == "expired"
	case "prepared":
		return to == "dispatched" || to == "rejected" || to == "expired"
	case "dispatched":
		return to == "observed" || to == "indeterminate" || to == "failed" || to == "rejected" || to == "delivered" || to == "delivery_failed"
	case "indeterminate":
		return to == "observed" || to == "delivered" || to == "failed"
	}
	return false
}
func (s *Store) TransitionCommand(ctx context.Context, t CommandTransition) (contracts.CommandRecord, error) {
	r := t.Record
	if !commandTransitionAllowed(t.From, r.State) {
		return r, contracts.ErrConflict
	}
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return r, err
	}
	defer tx.Rollback()
	var old contracts.CommandRecord
	var b []byte
	err = tx.QueryRowContext(ctx, "SELECT payload FROM commands WHERE id=?", r.Command.ID).Scan(&b)
	if err == nil {
		if json.Unmarshal(b, &old) != nil {
			return r, contracts.ErrCorrupt
		}
		if old.Principal != r.Principal || old.RequestHash != r.RequestHash {
			return old, &contracts.Error{Code: "id_reused", Message: "command ID already identifies another request"}
		}
		if old.State != t.From {
			return old, contracts.ErrConflict
		}
	} else if !errors.Is(err, sql.ErrNoRows) {
		return r, err
	} else if t.From != "" {
		return r, contracts.ErrConflict
	}
	if t.From == "" && err == nil {
		return old, contracts.ErrConflict
	}
	cursor, err := cursorAt(ctx, tx)
	if err != nil {
		return r, err
	}
	r.History = append(old.History, contracts.CommandEvent{State: r.State, At: time.Now().UTC(), Evidence: r.Evidence})
	cursor.Sequence++
	r.Cursor = cursor
	if (r.State == "observed" || r.State == "delivered") && (r.CommitSeq == 0 || r.CommitSeq >= cursor.Sequence) {
		return r, contracts.ErrCorrupt
	}
	b, err = json.Marshal(r)
	if err != nil {
		return r, err
	}
	_, err = tx.ExecContext(ctx, "INSERT INTO commands(id,principal,request_hash,state,payload) VALUES(?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,payload=excluded.payload", r.Command.ID, r.Principal, r.RequestHash, r.State, b)
	if err != nil {
		return r, err
	}
	record := contracts.Record{Collection: "system.command", ID: r.Command.ID, Version: uint64(len(r.History)), Source: r.Source, ObservedAt: time.Now().UTC(), Coverage: contracts.Coverage{Fields: "complete", Missing: []string{}}, Data: b}
	if err = writeRecord(ctx, tx, record, cursor.Sequence); err != nil {
		return r, err
	}
	if err = appendChange(ctx, tx, cursor, record, "upsert"); err != nil {
		return r, err
	}
	if err = setCursor(ctx, tx, cursor); err != nil {
		return r, err
	}
	if err = tx.Commit(); err != nil {
		return r, err
	}
	s.signal()
	return r, nil
}
func (s *Store) NativeKey(ctx context.Context, collection, id string) (string, error) {
	var key string
	err := s.db.QueryRowContext(ctx, "SELECT native_key FROM aliases WHERE collection=? AND record_id=? ORDER BY rowid DESC LIMIT 1", collection, id).Scan(&key)
	if errors.Is(err, sql.ErrNoRows) {
		return "", contracts.ErrNotFound
	}
	return key, err
}
