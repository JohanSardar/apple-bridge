package state

import (
	"context"
	"database/sql"
	"os"
	"path/filepath"
)

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
func migrate(db *sql.DB, home string) error {
	var schema string
	if err := db.QueryRow("SELECT value FROM meta WHERE key='schema_version'").Scan(&schema); err != nil || schema != "1" {
		return nil
	}
	backup := filepath.Join(home, "before-schema-2.db")
	valid := false
	if _, err := os.Stat(backup); err == nil {
		source, err := sql.Open("sqlite", "file:"+backup+"?mode=ro")
		if err == nil {
			var integrity string
			valid = source.QueryRow("PRAGMA integrity_check").Scan(&integrity) == nil && integrity == "ok"
			source.Close()
		}
	}
	if !valid {
		if err := atomicBackup(context.Background(), db, backup); err != nil {
			return err
		}
	}
	tx, err := db.Begin()
	if err != nil {
		return err
	}
	defer tx.Rollback()
	var columns int
	if err = tx.QueryRow("SELECT count(*) FROM pragma_table_info('journal') WHERE name='committed_at'").Scan(&columns); err != nil {
		return err
	}
	if columns == 0 {
		if _, err = tx.Exec("ALTER TABLE journal ADD COLUMN committed_at INTEGER NOT NULL DEFAULT 0"); err != nil {
			return err
		}
	}
	if _, err = tx.Exec("UPDATE journal SET committed_at=unixepoch() WHERE committed_at=0; UPDATE meta SET value='2' WHERE key='schema_version'"); err != nil {
		return err
	}
	return tx.Commit()
}
