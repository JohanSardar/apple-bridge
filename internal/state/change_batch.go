package state

import (
	"context"
	"database/sql"
	"encoding/json"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func readChangeBatchTx(ctx context.Context, tx *sql.Tx, visibility trust.Visibility, after contracts.Cursor) (contracts.Batch, error) {
	head, err := cursorAt(ctx, tx)
	if err != nil {
		return contracts.Batch{}, err
	}
	var floor uint64
	if err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='retention_floor'").Scan(&floor); err != nil {
		return contracts.Batch{}, err
	}
	if head.Epoch != after.Epoch || head.SubscriptionRevision != after.SubscriptionRevision || after.Sequence < floor {
		return contracts.Batch{}, contracts.ErrReset
	}
	batch := contracts.Batch{Epoch: after.Epoch, SubscriptionRevision: after.SubscriptionRevision, After: after.Sequence, Through: after.Sequence, Changes: []contracts.Change{}}
	if head.Sequence <= after.Sequence {
		return batch, nil
	}
	limit := min(head.Sequence, after.Sequence+128)
	var boundary sql.NullInt64
	if err = tx.QueryRowContext(ctx, "SELECT MAX(through_seq) FROM journal_commits WHERE epoch=? AND through_seq>? AND through_seq<=?", after.Epoch, after.Sequence, limit).Scan(&boundary); err != nil {
		return batch, err
	}
	if boundary.Valid {
		batch.Through = uint64(boundary.Int64)
	} else {
		batch.Through = limit
		batch.Partial = true
	}
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM journal WHERE seq>? AND seq<=? ORDER BY seq", batch.After, batch.Through)
	if err != nil {
		return batch, err
	}
	defer rows.Close()
	size := 0
	for rows.Next() {
		var raw []byte
		if err = rows.Scan(&raw); err != nil {
			return batch, err
		}
		var change contracts.Change
		if err = json.Unmarshal(raw, &change); err != nil {
			return batch, err
		}
		if !visibility.Allows(change.Record) {
			continue
		}
		if size+len(raw) > contracts.MaxFrame-2048 && len(batch.Changes) > 0 {
			rows.Close()
			if err = tx.QueryRowContext(ctx, "SELECT MAX(through_seq) FROM journal_commits WHERE epoch=? AND through_seq>? AND through_seq<?", after.Epoch, batch.After, change.Sequence).Scan(&boundary); err != nil {
				return batch, err
			}
			batch.Partial = !boundary.Valid
			batch.Through = change.Sequence - 1
			if boundary.Valid {
				batch.Through = uint64(boundary.Int64)
			}
			for len(batch.Changes) > 0 && batch.Changes[len(batch.Changes)-1].Sequence > batch.Through {
				batch.Changes = batch.Changes[:len(batch.Changes)-1]
			}
			return batch, nil
		}
		change.Record, err = projectContactLink(change.Record, visibility)
		if err != nil {
			return batch, err
		}
		batch.Changes = append(batch.Changes, change)
		size += len(raw)
	}
	return batch, rows.Err()
}
