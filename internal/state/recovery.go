package state

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) Backup(ctx context.Context, path string) error {
	s.write.Lock()
	defer s.write.Unlock()
	err := atomicBackup(ctx, s.db, path)
	if err == nil {
		err = os.Chmod(path, 0600)
	}
	return err
}

func Restore(ctx context.Context, home, backup string, acceptLedgerLoss ...bool) error {
	accept := len(acceptLedgerLoss) > 0 && acceptLedgerLoss[0]
	if backup == "" {
		if !accept {
			return contracts.ErrInvalid
		}
		backup = filepath.Join(home, "state.db")
	}

	absolute, err := filepath.Abs(backup)
	if err != nil {
		return err
	}
	source, err := sql.Open("sqlite", "file:"+absolute+"?mode=ro")
	if err != nil {
		return err
	}
	defer source.Close()
	var integrity string
	if err = source.QueryRowContext(ctx, "PRAGMA integrity_check").Scan(&integrity); err != nil || integrity != "ok" {
		return contracts.ErrCorrupt
	}
	if err = os.MkdirAll(home, 0700); err != nil {
		return err
	}
	temp, err := os.CreateTemp(home, "restore-*.db")
	if err != nil {
		return err
	}
	path := temp.Name()
	temp.Close()
	os.Remove(path)
	defer os.Remove(path)
	if _, err = source.ExecContext(ctx, "VACUUM INTO ?", path); err != nil {
		return err
	}
	if err = os.Chmod(path, 0600); err != nil {
		return err
	}
	staged, err := sql.Open("sqlite", "file:"+path)
	if err != nil {
		return err
	}
	if _, err = staged.ExecContext(ctx, "CREATE TABLE IF NOT EXISTS journal_commits(epoch TEXT NOT NULL,through_seq INTEGER NOT NULL,PRIMARY KEY(epoch,through_seq))"); err != nil {
		staged.Close()
		return err
	}
	tx, err := staged.BeginTx(ctx, nil)
	if err != nil {
		staged.Close()
		return err
	}
	cursor := contracts.Cursor{Epoch: contracts.NewID(""), SubscriptionRevision: 1}
	if err = setCursor(ctx, tx, cursor); err == nil {
		_, err = tx.ExecContext(ctx, "CREATE TABLE IF NOT EXISTS applied_batches(after_seq INTEGER PRIMARY KEY,through_seq INTEGER NOT NULL); DELETE FROM journal; DELETE FROM journal_commits; DELETE FROM staging_records; DELETE FROM applied_batches; UPDATE records SET seq=0; DELETE FROM meta WHERE key='replica_base'; UPDATE meta SET value='0' WHERE key='retention_floor'")
	}

	if err == nil {
		err = tx.Commit()
	} else {
		tx.Rollback()
	}
	staged.Close()
	if err != nil {
		return err
	}
	currentPath := filepath.Join(home, "state.db")
	if _, statErr := os.Stat(currentPath); statErr == nil {
		current, openErr := sql.Open("sqlite", "file:"+currentPath+"?_pragma=busy_timeout(5000)")
		if openErr != nil {
			return openErr
		}
		var busy, log, checkpointed int
		checkpointErr := current.QueryRowContext(ctx, "PRAGMA wal_checkpoint(TRUNCATE)").Scan(&busy, &log, &checkpointed)
		current.Close()
		if checkpointErr != nil {
			return checkpointErr
		}
		if busy != 0 {
			return contracts.ErrConflict
		}
	}
	for _, suffix := range []string{"-wal", "-shm"} {
		if err = os.Remove(filepath.Join(home, "state.db") + suffix); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	if err = rotateAdmissionEra(ctx, home, path, accept); err != nil {
		return err
	}
	if err = os.Rename(path, filepath.Join(home, "state.db")); err != nil {
		return err
	}
	dir, err := os.Open(home)
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}

type Integrity struct {
	Records     map[string]uint64 `json:"records"`
	OK          bool              `json:"ok"`
	Cursor      contracts.Cursor  `json:"cursor"`
	Applied     uint64            `json:"applied"`
	ExactlyOnce bool              `json:"exactly_once"`
	Floor       uint64            `json:"floor"`
}

func (s *Store) Integrity(ctx context.Context) (Integrity, error) {
	result := Integrity{Records: map[string]uint64{}}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return result, err
	}
	defer tx.Rollback()
	var check string
	if err = tx.QueryRowContext(ctx, "PRAGMA integrity_check").Scan(&check); err != nil || check != "ok" {
		return result, contracts.ErrCorrupt
	}
	result.Cursor, err = cursorAt(ctx, tx)
	if err != nil {
		return result, err
	}
	if err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='retention_floor'").Scan(&result.Floor); err != nil {
		return result, err
	}
	if err = tx.QueryRowContext(ctx, "SELECT count(*) FROM journal WHERE seq>? AND seq<=?", result.Floor, result.Cursor.Sequence).Scan(&result.Applied); err != nil {
		return result, err
	}
	var inconsistent int
	if err = tx.QueryRowContext(ctx, "SELECT count(*) FROM records WHERE seq>?", result.Cursor.Sequence).Scan(&inconsistent); err != nil {
		return result, err
	}
	var base uint64
	baseErr := tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='replica_base'").Scan(&base)
	result.ExactlyOnce = true
	if baseErr == nil {
		rows, err := tx.QueryContext(ctx, "SELECT after_seq,through_seq FROM applied_batches ORDER BY after_seq")
		if err != nil {
			return result, err
		}
		next := base
		for rows.Next() {
			var after, through uint64
			if err = rows.Scan(&after, &through); err != nil {
				rows.Close()
				return result, err
			}
			if after != next || through <= after {
				result.ExactlyOnce = false
			}
			next = through
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return result, err
		}
		result.ExactlyOnce = result.ExactlyOnce && next == result.Cursor.Sequence
	}
	rows, err := tx.QueryContext(ctx, "SELECT collection,count(*) FROM records WHERE deleted=0 GROUP BY collection")
	if err != nil {
		return result, err
	}
	for rows.Next() {
		var collection string
		var count uint64
		if err = rows.Scan(&collection, &count); err != nil {
			rows.Close()
			return result, err
		}
		result.Records[collection] = count
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return result, err
	}
	result.OK = inconsistent == 0 && result.ExactlyOnce
	return result, nil
}

func InspectHome(home string) (*Store, error) {
	path, err := filepath.Abs(filepath.Join(home, "state.db"))
	if err != nil {
		return nil, err
	}
	db, err := sql.Open("sqlite", "file:"+path+"?mode=ro&_pragma=busy_timeout(5000)")
	if err != nil {
		return nil, err
	}
	return &Store{db: db}, nil
}
