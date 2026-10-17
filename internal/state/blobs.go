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
	rows, err := q.QueryContext(ctx, `SELECT payload FROM records WHERE deleted=0 AND (instr(payload,'"digest"')>0 OR instr(payload,'"raw_body_digest"')>0 OR instr(payload,'\u')>0) UNION ALL SELECT payload FROM staging_records WHERE (instr(payload,'"digest"')>0 OR instr(payload,'"raw_body_digest"')>0 OR instr(payload,'\u')>0)`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var b []byte
		if err = rows.Scan(&b); err != nil {
			return nil, err
		}
		var r contracts.Record
		if err = json.Unmarshal(b, &r); err != nil {
			return nil, err
		}
		refs["record:"+r.ID] = append(refs["record:"+r.ID], contracts.BlobReferences(r.Data)...)
	}
	if err = rows.Err(); err != nil {
		return nil, err
	}
	rows.Close()
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
		refs[owner] = list
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
