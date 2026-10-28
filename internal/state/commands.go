package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type CommandTransition struct {
	From   contracts.CommandState
	Record contracts.CommandRecord
}

func (s *Store) checkLedger(ctx context.Context) error {
	path := filepath.Join(s.home, "ledger.db")
	if _, err := os.Stat(path); os.IsNotExist(err) {
		var prior int
		if err = s.db.QueryRowContext(ctx, "SELECT count(*) FROM records WHERE collection='system.command'").Scan(&prior); err != nil {
			return err
		}
		var marked int
		if err = s.db.QueryRowContext(ctx, "SELECT count(*) FROM meta WHERE key='commands_seen'").Scan(&marked); err != nil {
			return err
		}
		var legacy int
		if err = s.db.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_master WHERE name='commands'").Scan(&legacy); err != nil {
			return err
		}
		if legacy > 0 {
			if err = s.db.QueryRowContext(ctx, "SELECT count(*) FROM commands").Scan(&legacy); err != nil {
				return err
			}
		}
		if prior > 0 || marked > 0 || legacy > 0 {
			return &contracts.Error{Code: "ledger_missing", Message: "command ledger missing; run bridge restore --accept-ledger-loss"}
		}
	} else if err != nil {
		return err
	}
	return nil
}
func (s *Store) InitCommands(ctx context.Context) error {
	if err := s.checkLedger(ctx); err != nil {
		return err
	}
	return s.openLedger(ctx)
}
func (s *Store) openLedger(ctx context.Context, publishMirror ...bool) error {
	s.write.Lock()
	defer s.write.Unlock()
	path := filepath.Join(s.home, "ledger.db")
	file, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	file.Close()
	db, err := sql.Open("sqlite", "file:"+path+"?_pragma=journal_mode(WAL)&_pragma=synchronous(FULL)&_pragma=busy_timeout(5000)")
	if err != nil {
		return err
	}
	db.SetMaxOpenConns(1)
	dir, err := os.Open(s.home)
	if err != nil {
		db.Close()
		return err
	}
	err = dir.Sync()
	dir.Close()
	if err != nil {
		db.Close()
		return err
	}
	s.ledger = db
	_, err = db.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS commands(id TEXT PRIMARY KEY,principal TEXT NOT NULL,request_hash TEXT NOT NULL,state TEXT NOT NULL,payload BLOB NOT NULL);
 CREATE INDEX IF NOT EXISTS commands_state ON commands(state);
 CREATE TABLE IF NOT EXISTS dedupe(command_id TEXT PRIMARY KEY,principal TEXT NOT NULL,request_hash TEXT NOT NULL,admission_era TEXT NOT NULL);
 INSERT OR IGNORE INTO meta(key,value) VALUES('admission_era','`+contracts.NewID("")+`');`)
	if err != nil {
		return err
	}
	era, err := s.AdmissionEra(ctx)
	if err != nil {
		return err
	}
	if _, err = s.db.ExecContext(ctx, "INSERT INTO meta(key,value) VALUES('admission_era',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", era); err != nil {
		return err
	}
	if len(publishMirror) > 0 && !publishMirror[0] {
		return nil
	}
	records, err := s.Commands(ctx)
	if err != nil {
		return err
	}
	for _, r := range records {
		var version uint64
		err := s.db.QueryRowContext(ctx, "SELECT version FROM records WHERE collection='system.command' AND id=?", r.Command.ID).Scan(&version)
		if err != nil && !errors.Is(err, sql.ErrNoRows) {
			return err
		}
		if version != uint64(len(r.History)) {
			if err = s.publishCommandLocked(ctx, r); err != nil {
				return err
			}
		}
	}
	return nil
}
func (s *Store) AdmissionEra(ctx context.Context) (string, error) {
	var era string
	err := s.ledger.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='admission_era'").Scan(&era)
	return era, err
}
func (s *Store) Command(ctx context.Context, id string) (contracts.CommandRecord, error) {
	var r contracts.CommandRecord
	var b []byte
	err := s.ledger.QueryRowContext(ctx, "SELECT payload FROM commands WHERE id=?", id).Scan(&b)
	if errors.Is(err, sql.ErrNoRows) {
		return r, contracts.ErrNotFound
	}
	if err == nil {
		err = json.Unmarshal(b, &r)
	}
	return r, err
}
func (s *Store) Commands(ctx context.Context) ([]contracts.CommandRecord, error) {
	rows, err := s.ledger.QueryContext(ctx, "SELECT payload FROM commands ORDER BY id")
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
func commandTransitionAllowed(from, to contracts.CommandState) bool {
	switch from {
	case "":
		return to == contracts.CommandAdmitted || to == contracts.CommandAwaitingApproval
	case contracts.CommandAwaitingApproval:
		return to == contracts.CommandAdmitted || to == contracts.CommandRejectedByOwner || to == contracts.CommandExpired || to == contracts.CommandRejected
	case contracts.CommandAdmitted:
		return to == contracts.CommandPrepared || to == contracts.CommandRejected || to == contracts.CommandExpired
	case contracts.CommandPrepared:
		return to == contracts.CommandDispatched || to == contracts.CommandRejected || to == contracts.CommandExpired
	case contracts.CommandDispatched:
		return to == contracts.CommandObserved || to == contracts.CommandIndeterminate || to == contracts.CommandFailed || to == contracts.CommandRejected || to == contracts.CommandDelivered || to == contracts.CommandDeliveryFailed
	case contracts.CommandObserved:
		return to == contracts.CommandDelivered || to == contracts.CommandDeliveryFailed
	case contracts.CommandIndeterminate:
		return to == contracts.CommandObserved || to == contracts.CommandDelivered || to == contracts.CommandFailed || to == contracts.CommandDeliveryFailed
	}
	return false
}
func (s *Store) TransitionCommand(ctx context.Context, t CommandTransition) (contracts.CommandRecord, error) {
	return s.persistCommand(ctx, t, false)
}
func (s *Store) UpdateCommandParts(ctx context.Context, r contracts.CommandRecord) (contracts.CommandRecord, error) {
	return s.persistCommand(ctx, CommandTransition{From: r.State, Record: r}, true)
}
func (s *Store) persistCommand(ctx context.Context, t CommandTransition, partsUpdated bool) (contracts.CommandRecord, error) {
	r := t.Record
	if partsUpdated && (t.From != r.State || r.Command.Op != "messages.send") || !partsUpdated && !commandTransitionAllowed(t.From, r.State) {
		return r, contracts.ErrConflict
	}
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.ledger.BeginTx(ctx, nil)
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
	changed, valid := commandPartsChange(old.Parts, r.Parts)
	if !valid || partsUpdated && (!changed || r.Disposition != old.Disposition) {
		return old, contracts.ErrConflict
	}
	if t.From == "" && err == nil {
		return old, contracts.ErrConflict
	}
	if t.From == "" {
		var era string
		if err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='admission_era'").Scan(&era); err != nil {
			return r, err
		}
		if r.AdmissionEra != era {
			return r, contracts.ErrExpired
		}
	}
	cursor, err := cursorAt(ctx, s.db)
	if err != nil {
		return r, err
	}
	event := contracts.CommandEvent{State: r.State, At: time.Now().UTC(), Evidence: r.Evidence}
	if partsUpdated {
		if cursor.Epoch != old.Cursor.Epoch || cursor.Sequence <= old.CommitSeq {
			return old, contracts.ErrConflict
		}
		r.CommitSeq = cursor.Sequence
		event.Transition = "parts_updated"
	}
	r.History = append(old.History, event)
	cursor.Sequence++
	r.Cursor = cursor
	runnerCompletion := (r.Command.Op == "shortcuts.run" && r.Source == contracts.Shortcuts && contracts.ShortcutCompletionEvidence(r.Evidence, r.RequestHash) || r.Source == "music" && contracts.MusicPlayerOperation(r.Command.Op) && contracts.MusicCompletionEvidence(r.Evidence, r.RequestHash)) && r.State == contracts.CommandObserved && r.CommitSeq == 0
	if !runnerCompletion && (r.State == contracts.CommandObserved || r.State == contracts.CommandDelivered) && (r.CommitSeq == 0 || r.CommitSeq >= cursor.Sequence) {
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
	if t.From == "" {
		if _, err = tx.ExecContext(ctx, "INSERT INTO dedupe(command_id,principal,request_hash,admission_era) VALUES(?,?,?,?)", r.Command.ID, r.Principal, r.RequestHash, r.AdmissionEra); err != nil {
			return r, err
		}
		if _, err = s.db.ExecContext(ctx, "INSERT OR IGNORE INTO meta(key,value) VALUES('commands_seen','1')"); err != nil {
			return r, err
		}
	}
	if err = tx.Commit(); err != nil {
		return r, err
	}
	if err = s.publishCommandLocked(ctx, r); err != nil {
		return r, err
	}
	return r, nil
}
func (s *Store) publishCommandLocked(ctx context.Context, r contracts.CommandRecord) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if trust.IsAgentPrincipalID(r.Principal) {
		if err = writeAgentActivity(ctx, tx, AgentActivity{Sequence: r.Cursor.Sequence, ID: r.Principal + ":" + r.Command.ID, Principal: r.Principal, Operation: r.Command.Op, Kind: "command", State: string(r.State), At: r.CreatedAt, ArgumentBytes: len(r.Command.Args), ArgumentHash: trust.Fingerprint(r.Command.Args)}); err != nil {
			return err
		}
	}
	cursor, err := cursorAt(ctx, tx)
	if err != nil {
		return err
	}
	cursor.Sequence++
	b, err := json.Marshal(struct {
		CommandID        string `json:"command_id"`
		Principal        string `json:"principal"`
		AwaitingApproval bool   `json:"awaiting_approval,omitempty"`
	}{r.Command.ID, r.Principal, r.State == contracts.CommandAwaitingApproval})
	if err != nil {
		return err
	}
	record := contracts.Record{Collection: "system.command", ID: r.Command.ID, Version: uint64(len(r.History)), Source: r.Source, ObservedAt: r.History[len(r.History)-1].At, Coverage: contracts.Coverage{Fields: "complete", Missing: []string{}}, Data: b}
	raw, err := json.Marshal(r)
	if err != nil {
		return err
	}
	refs, err := json.Marshal(contracts.BlobReferences(raw))
	if err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO blob_reference_sets(owner,payload) VALUES(?,?) ON CONFLICT(owner) DO UPDATE SET payload=excluded.payload", "command:"+r.Command.ID, refs); err != nil {
		return err
	}
	if err = writeRecord(ctx, tx, record, cursor.Sequence); err != nil {
		return err
	}
	if err = appendChange(ctx, tx, cursor, record, "upsert"); err != nil {
		return err
	}
	if err = setCursor(ctx, tx, cursor); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "INSERT OR IGNORE INTO meta(key,value) VALUES('commands_seen','1')"); err != nil {
		return err
	}
	if err = s.commitProtected(ctx, tx); err != nil {
		return err
	}
	s.signal()
	return nil
}

func (s *Store) NativeKey(ctx context.Context, collection, id string) (string, error) {
	var key string
	err := s.db.QueryRowContext(ctx, "SELECT native_key FROM aliases WHERE collection=? AND record_id=? ORDER BY rowid DESC LIMIT 1", collection, id).Scan(&key)
	if errors.Is(err, sql.ErrNoRows) {
		return "", contracts.ErrNotFound
	}
	return key, err
}

func (s *Store) ObservedRecordID(ctx context.Context, collection, nativeKey string, cursor contracts.Cursor) (string, error) {
	var id string
	err := s.db.QueryRowContext(ctx, `SELECT a.record_id FROM aliases a
 JOIN checkpoints c ON c.source=a.source AND c.generation=a.generation
 JOIN records r ON r.collection=a.collection AND r.id=a.record_id
 WHERE a.source=? AND a.collection=? AND a.native_key=?
 AND EXISTS(SELECT 1 FROM meta WHERE key='epoch' AND value=?)`, contracts.CollectionSource(collection), collection, nativeKey, cursor.Epoch).Scan(&id)
	if errors.Is(err, sql.ErrNoRows) {
		return "", contracts.ErrNotFound
	}
	return id, err
}

func rotateAdmissionEra(ctx context.Context, home, stagedPath string, accept bool) error {
	if !accept {
		if _, err := os.Stat(filepath.Join(home, "state.db")); err == nil {
			db, err := sql.Open("sqlite", "file:"+filepath.Join(home, "state.db")+"?_pragma=busy_timeout(5000)")
			if err != nil {
				return err
			}
			current := &Store{home: home, db: db, changed: make(chan struct{})}
			err = current.checkLedger(ctx)
			current.Close()
			if err != nil {
				return err
			}
		}
	}
	staged, err := sql.Open("sqlite", "file:"+stagedPath)
	if err != nil {
		return err
	}
	store := &Store{home: home, db: staged, changed: make(chan struct{})}
	defer store.Close()
	if !accept {
		if err = store.checkLedger(ctx); err != nil {
			return err
		}
	}
	err = store.openLedger(ctx, false)
	if err != nil {
		return err
	}
	era := contracts.NewID("")
	if _, err = store.ledger.ExecContext(ctx, "UPDATE meta SET value=? WHERE key='admission_era'", era); err != nil {
		return err
	}
	_, err = staged.ExecContext(ctx, "INSERT INTO meta(key,value) VALUES('admission_era',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", era)
	return err
}

func (s *Store) PendingApprovals(ctx context.Context) ([]contracts.CommandRecord, error) {
	rows, err := s.ledger.QueryContext(ctx, "SELECT payload FROM commands WHERE state='awaiting_approval' ORDER BY id")
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []contracts.CommandRecord{}
	for rows.Next() {
		var raw []byte
		var r contracts.CommandRecord
		if err = rows.Scan(&raw); err != nil {
			return nil, err
		}
		if err = json.Unmarshal(raw, &r); err != nil {
			return nil, err
		}
		out = append(out, r)
	}
	return out, rows.Err()
}
