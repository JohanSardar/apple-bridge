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
	tx, err := staged.BeginTx(ctx, nil)
	if err != nil {
		staged.Close()
		return err
	}
	cursor := contracts.Cursor{Epoch: contracts.NewID(""), SubscriptionRevision: 1}
	if err = setCursor(ctx, tx, cursor); err == nil {
		_, err = tx.ExecContext(ctx, "CREATE TABLE IF NOT EXISTS applied_batches(after_seq INTEGER PRIMARY KEY,through_seq INTEGER NOT NULL); DELETE FROM journal; DELETE FROM staging_records; DELETE FROM applied_batches; UPDATE records SET seq=0; DELETE FROM meta WHERE key='replica_base'; UPDATE meta SET value='0' WHERE key='retention_floor'")
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

func atomicBackup(ctx context.Context, db *sql.DB, path string) error {
	temp, err := os.CreateTemp(filepath.Dir(path), "backup-*.db")
	if err != nil {
		return err
	}
	staging := temp.Name()
	temp.Close()
	os.Remove(staging)
	defer os.Remove(staging)
	if _, err = db.ExecContext(ctx, "VACUUM INTO ?", staging); err != nil {
		return err
	}
	file, err := os.OpenFile(staging, os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	err = file.Chmod(0600)
	if err == nil {
		err = file.Sync()
	}
	file.Close()
	if err != nil {
		return err
	}
	if err = os.Rename(staging, path); err != nil {
		return err
	}
	dir, err := os.Open(filepath.Dir(path))
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
