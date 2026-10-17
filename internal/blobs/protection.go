package blobs

import (
	"context"
	"database/sql"
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	_ "modernc.org/sqlite"
)

func (s *Store) openProtection(ctx context.Context) error {
	path := filepath.Join(s.root, "protection.db")
	f, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	f.Close()
	db, err := sql.Open("sqlite", path)
	if err != nil {
		return err
	}
	s.protection = db
	db.SetMaxOpenConns(1)
	if _, err = db.ExecContext(ctx, `PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA busy_timeout=5000;
 CREATE TABLE IF NOT EXISTS protection_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS protection_owners(owner TEXT PRIMARY KEY,payload BLOB NOT NULL);
 CREATE TABLE IF NOT EXISTS protection_unreferenced(digest TEXT PRIMARY KEY,at TEXT NOT NULL);
 CREATE TABLE IF NOT EXISTS protection_released_pins(pin TEXT PRIMARY KEY);`); err != nil {
		return err
	}
	var initialized int
	if err = db.QueryRowContext(ctx, "SELECT count(*) FROM protection_meta WHERE key='initialized'").Scan(&initialized); err != nil {
		return err
	}
	if initialized == 0 {
		tx, err := db.BeginTx(ctx, nil)
		if err != nil {
			return err
		}
		defer tx.Rollback()
		for owner, refs := range s.manifest.References {
			b, err := json.Marshal(refs)
			if err != nil {
				return err
			}
			if _, err = tx.ExecContext(ctx, "INSERT INTO protection_owners(owner,payload) VALUES(?,?)", owner, b); err != nil {
				return err
			}
		}
		for d, e := range s.manifest.Entries {
			if !e.UnreferencedAt.IsZero() {
				if _, err = tx.ExecContext(ctx, "INSERT INTO protection_unreferenced(digest,at) VALUES(?,?)", d, e.UnreferencedAt.Format(time.RFC3339Nano)); err != nil {
					return err
				}
			}
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO protection_meta(key,value) VALUES('initialized','1')"); err != nil {
			return err
		}
		if err = tx.Commit(); err != nil {
			return err
		}
	}
	s.manifest.References = contracts.ReferenceSet{}
	s.referenceCounts = map[string]int{}
	s.digestPins = map[string]map[string]bool{}
	rows, err := db.QueryContext(ctx, "SELECT owner,payload FROM protection_owners")
	if err != nil {
		return err
	}
	for rows.Next() {
		var owner string
		var b []byte
		if err = rows.Scan(&owner, &b); err != nil {
			rows.Close()
			return err
		}
		var refs []contracts.BlobReference
		if err = json.Unmarshal(b, &refs); err != nil {
			rows.Close()
			return err
		}
		s.manifest.References[owner] = refs
		for d, ref := range uniqueReferences(refs) {
			if !contracts.ValidDigest(d) {
				rows.Close()
				return contracts.ErrCorrupt
			}
			s.referenceCounts[d]++
			e, ok := s.manifest.Entries[d]
			if !ok {
				e = referenceEntry(ref)
			}
			e.UnreferencedAt = time.Time{}
			s.manifest.Entries[d] = e
		}
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	rows, err = db.QueryContext(ctx, "SELECT digest,at FROM protection_unreferenced")
	if err != nil {
		return err
	}
	for rows.Next() {
		var d, at string
		if err = rows.Scan(&d, &at); err != nil {
			rows.Close()
			return err
		}
		if e, ok := s.manifest.Entries[d]; ok && s.referenceCounts[d] == 0 {
			e.UnreferencedAt, err = time.Parse(time.RFC3339Nano, at)
			if err != nil {
				rows.Close()
				return err
			}
			s.manifest.Entries[d] = e
		}
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	rows, err = db.QueryContext(ctx, "SELECT pin FROM protection_released_pins")
	if err != nil {
		return err
	}
	for rows.Next() {
		var pin string
		if err = rows.Scan(&pin); err != nil {
			rows.Close()
			return err
		}
		delete(s.manifest.Pins, pin)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return err
	}
	for pin, d := range s.manifest.Pins {
		if s.digestPins[d] == nil {
			s.digestPins[d] = map[string]bool{}
		}
		s.digestPins[d][pin] = true
	}
	return nil
}

func uniqueReferences(refs []contracts.BlobReference) map[string]contracts.BlobReference {
	out := map[string]contracts.BlobReference{}
	for _, ref := range refs {
		out[ref.Digest] = ref
	}
	return out
}

func referenceEntry(ref contracts.BlobReference) Entry {
	state := "pending"
	if ref.State == "offloaded" || ref.State == "missing" {
		state = ref.State
	}
	return Entry{State: state, Size: ref.Size}
}

func (s *Store) Close() error {
	if s.protection == nil {
		return nil
	}
	return s.protection.Close()
}

func (s *Store) UpdateProtection(ctx context.Context, updates contracts.ReferenceSet, releasePins bool) error {
	if err := ctx.Err(); err != nil {
		return err
	}
	if len(updates) == 0 {
		return nil
	}
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.updateProtectionLocked(ctx, updates, releasePins)
}

func (s *Store) updateProtectionLocked(ctx context.Context, updates contracts.ReferenceSet, releasePins bool) error {
	counts := map[string]int{}
	entries := map[string]Entry{}
	for owner, refs := range updates {
		for _, ref := range refs {
			if !contracts.ValidDigest(ref.Digest) {
				return contracts.ErrInvalid
			}
		}
		for d := range uniqueReferences(s.manifest.References[owner]) {
			if _, ok := counts[d]; !ok {
				counts[d] = s.referenceCounts[d]
			}
			counts[d]--
		}
		for d, ref := range uniqueReferences(refs) {
			if _, ok := counts[d]; !ok {
				counts[d] = s.referenceCounts[d]
			}
			counts[d]++
			if _, ok := s.manifest.Entries[d]; !ok {
				entries[d] = referenceEntry(ref)
			}
		}
	}
	for _, count := range counts {
		if count < 0 {
			return contracts.ErrCorrupt
		}
	}
	tx, err := s.protection.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	for owner, refs := range updates {
		if len(refs) == 0 {
			_, err = tx.ExecContext(ctx, "DELETE FROM protection_owners WHERE owner=?", owner)
		} else {
			var b []byte
			b, err = json.Marshal(refs)
			if err == nil {
				_, err = tx.ExecContext(ctx, "INSERT INTO protection_owners(owner,payload) VALUES(?,?) ON CONFLICT(owner) DO UPDATE SET payload=excluded.payload", owner, b)
			}
		}
		if err != nil {
			return err
		}
	}
	released := map[string]string{}
	now := time.Now().UTC()
	for d, count := range counts {
		e, exists := s.manifest.Entries[d]
		if !exists {
			e, exists = entries[d]
		}
		if !exists {
			continue
		}
		if count > 0 {
			e.UnreferencedAt = time.Time{}
			_, err = tx.ExecContext(ctx, "DELETE FROM protection_unreferenced WHERE digest=?", d)
		} else {
			if e.UnreferencedAt.IsZero() {
				e.UnreferencedAt = now
			}
			_, err = tx.ExecContext(ctx, "INSERT INTO protection_unreferenced(digest,at) VALUES(?,?) ON CONFLICT(digest) DO UPDATE SET at=excluded.at", d, e.UnreferencedAt.Format(time.RFC3339Nano))
		}
		if err != nil {
			return err
		}
		entries[d] = e
		if releasePins && count > 0 {
			for pin := range s.digestPins[d] {
				if _, err = tx.ExecContext(ctx, "INSERT OR IGNORE INTO protection_released_pins(pin) VALUES(?)", pin); err != nil {
					return err
				}
				released[pin] = d
			}
		}
	}
	if err = tx.Commit(); err != nil {
		return err
	}
	for owner, refs := range updates {
		if len(refs) == 0 {
			delete(s.manifest.References, owner)
		} else {
			s.manifest.References[owner] = refs
		}
	}
	for d, count := range counts {
		if count == 0 {
			delete(s.referenceCounts, d)
		} else {
			s.referenceCounts[d] = count
		}
	}
	for d, e := range entries {
		s.manifest.Entries[d] = e
	}
	for pin, d := range released {
		delete(s.manifest.Pins, pin)
		delete(s.digestPins[d], pin)
		if len(s.digestPins[d]) == 0 {
			delete(s.digestPins, d)
		}
	}
	return nil
}
