package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Store) RecordGrant(ctx context.Context, v trust.Visibility, id string) (trust.RecordGrant, error) {
	if id == "" {
		return trust.RecordGrant{}, contracts.ErrUnauthorized
	}
	result, err := s.Read(ctx, v, contracts.ReadQuery{ID: id})
	if err != nil || len(result.Records) != 1 {
		return trust.RecordGrant{}, contracts.ErrUnauthorized
	}
	return trust.GrantRecord(result.Records[0]), nil
}
func blobReferences(ctx context.Context, q interface {
	QueryContext(context.Context, string, ...any) (*sql.Rows, error)
	QueryRowContext(context.Context, string, ...any) *sql.Row
}) (contracts.ReferenceSet, error) {
	refs := contracts.ReferenceSet{}
	rows, err := q.QueryContext(ctx, "SELECT owner,payload FROM blob_refs")
	if err != nil {
		return nil, err
	}
	for rows.Next() {
		var owner string
		var b []byte
		if err = rows.Scan(&owner, &b); err != nil {
			rows.Close()
			return nil, err
		}
		var ref contracts.BlobReference
		if err = json.Unmarshal(b, &ref); err != nil {
			rows.Close()
			return nil, err
		}
		refs[owner] = append(refs[owner], ref)
	}
	err = rows.Err()
	rows.Close()
	if err != nil {
		return nil, err
	}

	extra, err := q.QueryContext(ctx, "SELECT owner,payload FROM blob_reference_sets")
	if err != nil {
		return nil, err
	}
	for extra.Next() {
		var owner string
		var b []byte
		if err = extra.Scan(&owner, &b); err != nil {
			extra.Close()
			return nil, err
		}
		var list []contracts.BlobReference
		if err = json.Unmarshal(b, &list); err != nil {
			extra.Close()
			return nil, err
		}
		if len(list) > 0 {
			refs[owner] = list
		}
	}
	err = extra.Err()
	extra.Close()
	if err != nil {
		return nil, err
	}
	var count int
	if err = q.QueryRowContext(ctx, "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='commands'").Scan(&count); err != nil {
		return nil, err
	}
	if count > 0 {
		commands, err := q.QueryContext(ctx, "SELECT id,payload FROM commands")
		if err != nil {
			return nil, err
		}
		defer commands.Close()
		for commands.Next() {
			var id string
			var b []byte
			if err = commands.Scan(&id, &b); err != nil {
				return nil, err
			}
			refs["command:"+id] = contracts.BlobReferences(b)
		}
		if err = commands.Err(); err != nil {
			return nil, err
		}
	}
	return refs, nil
}
func (s *Store) protectTransaction(ctx context.Context, tx *sql.Tx) error {
	if s.BlobProtection == nil {
		return nil
	}
	next, err := blobReferences(ctx, tx)
	if err != nil {
		return err
	}
	previous, err := blobReferences(ctx, s.db)
	if err != nil {
		return err
	}
	for k, list := range previous {
		next["previous:"+k] = list
	}
	for k, list := range s.blobSnapshots {
		next["snapshot:"+k] = list
	}
	if s.BlobPreparation != nil {
		return s.BlobPreparation(ctx, next)
	}
	return s.BlobProtection(ctx, next)
}
func (s *Store) refreshBlobProtection(ctx context.Context) error {
	if s.BlobProtection == nil {
		return nil
	}
	refs, err := blobReferences(ctx, s.db)
	if err != nil {
		return err
	}
	for k, list := range s.blobSnapshots {
		refs["snapshot:"+k] = list
	}
	return s.BlobProtection(ctx, refs)
}
func (s *Store) ProtectBlobs(ctx context.Context) error {
	s.write.Lock()
	defer s.write.Unlock()
	return s.refreshBlobProtection(ctx)
}

func (s *Store) BlobReferenceSet(ctx context.Context) (contracts.ReferenceSet, error) {
	s.write.Lock()
	defer s.write.Unlock()
	refs, err := blobReferences(ctx, s.db)
	if err != nil {
		return nil, err
	}
	for k, list := range s.blobSnapshots {
		refs["snapshot:"+k] = list
	}
	return refs, nil
}
func (s *Store) BlobGrant(ctx context.Context, v trust.Visibility, digest string) (trust.RecordGrant, error) {
	after := ""
	for {
		result, err := s.Read(ctx, v, contracts.ReadQuery{After: after})
		if err != nil {
			return trust.RecordGrant{}, err
		}
		for _, r := range result.Records {
			g := trust.GrantRecord(r)
			if g.AllowsBlob(digest) {
				return g, nil
			}
		}
		if result.Next == "" {
			return trust.RecordGrant{}, contracts.ErrUnauthorized
		}
		after = result.Next
	}
}

func (s *Store) SetBlobReferences(ctx context.Context, owner string, refs []contracts.BlobReference) error {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if len(refs) == 0 {
		_, err = tx.ExecContext(ctx, "DELETE FROM blob_reference_sets WHERE owner=?", owner)
	} else {
		var b []byte
		b, err = json.Marshal(refs)
		if err == nil {
			_, err = tx.ExecContext(ctx, "INSERT INTO blob_reference_sets(owner,payload) VALUES(?,?) ON CONFLICT(owner) DO UPDATE SET payload=excluded.payload", owner, b)
		}
	}
	if err != nil {
		return err
	}
	if err = s.protectTransaction(ctx, tx); err != nil {
		return err
	}
	if err = tx.Commit(); err != nil {
		return err
	}
	return s.refreshBlobProtection(ctx)
}

func writeBlobReferences(ctx context.Context, tx *sql.Tx, owner string, r contracts.Record) error {
	if _, err := tx.ExecContext(ctx, "DELETE FROM blob_refs WHERE owner=?", owner); err != nil {
		return err
	}
	if r.Deleted {
		return nil
	}
	for _, ref := range contracts.BlobReferences(r.Data) {
		b, err := json.Marshal(ref)
		if err != nil {
			return err
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO blob_refs(owner,digest,payload) VALUES(?,?,?) ON CONFLICT(owner,digest) DO UPDATE SET payload=excluded.payload", owner, ref.Digest, b); err != nil {
			return err
		}
	}
	return nil
}
func (s *Store) initBlobReferences(ctx context.Context) error {
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if _, err = tx.ExecContext(ctx, `CREATE TABLE IF NOT EXISTS blob_refs(owner TEXT NOT NULL,digest TEXT NOT NULL,payload BLOB NOT NULL,PRIMARY KEY(owner,digest));
 CREATE TRIGGER IF NOT EXISTS blob_refs_record_delete AFTER DELETE ON records BEGIN DELETE FROM blob_refs WHERE owner='record:'||OLD.id; END;
 CREATE TRIGGER IF NOT EXISTS blob_refs_staging_delete AFTER DELETE ON staging_records BEGIN DELETE FROM blob_refs WHERE owner='staging:'||OLD.snapshot||':'||OLD.id; END;`); err != nil {
		return err
	}
	var ready int
	if err = tx.QueryRowContext(ctx, "SELECT count(*) FROM meta WHERE key='blob_refs_version'").Scan(&ready); err != nil {
		return err
	}
	if ready == 0 {
		rows, err := tx.QueryContext(ctx, "SELECT payload FROM records WHERE deleted=0")
		if err != nil {
			return err
		}
		var records []contracts.Record
		for rows.Next() {
			var b []byte
			var r contracts.Record
			if err = rows.Scan(&b); err == nil {
				err = json.Unmarshal(b, &r)
			}
			if err != nil {
				rows.Close()
				return err
			}
			records = append(records, r)
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return err
		}
		for _, r := range records {
			if err = writeBlobReferences(ctx, tx, "record:"+r.ID, r); err != nil {
				return err
			}
		}
		if _, err = tx.ExecContext(ctx, "INSERT INTO meta(key,value) VALUES('blob_refs_version','1')"); err != nil {
			return err
		}
	}
	return tx.Commit()
}
