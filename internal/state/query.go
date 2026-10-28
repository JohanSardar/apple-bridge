package state

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"time"
)

func (s *Store) SaveQueryBlobs(ctx context.Context, principal, id string, scope contracts.Grant, blobs []contracts.QueryBlob) error {
	payload, _ := json.Marshal(blobs)
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	_, err = tx.ExecContext(ctx, `INSERT INTO query_blobs VALUES(?,?,?,?,?)`, id, principal, scope, payload, time.Now().Add(time.Hour).Unix())
	if err != nil {
		return err
	}
	var refs []contracts.BlobReference
	for _, b := range blobs {
		refs = append(refs, contracts.BlobReference{Digest: b.Digest, Size: b.Size, State: "present"})
	}
	raw, _ := json.Marshal(refs)
	_, err = tx.ExecContext(ctx, `DELETE FROM blob_reference_sets WHERE owner IN (SELECT id FROM query_blobs WHERE expires<?)`, time.Now().Unix())
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `DELETE FROM query_blobs WHERE expires<?`, time.Now().Unix())
	if err != nil {
		return err
	}
	_, err = tx.ExecContext(ctx, `INSERT INTO blob_reference_sets(owner,payload) VALUES(?,?)`, id, raw)
	if err != nil {
		return err
	}
	return s.commitProtected(ctx, tx)
}
func (s *Store) QueryBlobGrant(ctx context.Context, p trust.Principal, id string) (trust.RecordGrant, error) {
	var scope contracts.Grant
	var payload []byte
	err := s.db.QueryRowContext(ctx, `SELECT scope,payload FROM query_blobs WHERE id=? AND principal=? AND expires>?`, id, p.ID, time.Now().Unix()).Scan(&scope, &payload)
	if err != nil || !trust.Subset([]contracts.Grant{scope}, p.Grants) {
		return trust.RecordGrant{}, contracts.ErrForbidden
	}
	return trust.GrantRecord(contracts.Record{ID: id, Data: payload}), nil
}
