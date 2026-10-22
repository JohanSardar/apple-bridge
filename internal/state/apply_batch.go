package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func clearPendingCapture(ctx context.Context, tx *sql.Tx) error {
	_, err := tx.ExecContext(ctx, "DELETE FROM pending_changes; DELETE FROM blob_refs WHERE owner LIKE 'capture-stage:%'")
	return err
}
func (s *Store) DiscardCapture(ctx context.Context) error {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err = clearPendingCapture(ctx, tx); err != nil {
		return err
	}
	if err = s.commitProtected(ctx, tx); err != nil {
		return err
	}
	captureDiscarded(s.home, s.pendingCapture != nil)
	s.pendingCapture = nil
	return nil
}
func validateBatchChange(change contracts.Change, previous, through uint64) error {
	if change.Sequence <= previous || change.Sequence > through || change.Record.Version == 0 || change.Record.ID == "" || (change.Op != "upsert" && change.Op != "delete" && change.Op != "evict") {
		return contracts.ErrCorrupt
	}
	return nil
}
func (w *recordWriter) applyBatchChange(ctx context.Context, epoch string, change contracts.Change) error {
	if err := w.writeRecord(ctx, change.Record, change.Sequence); err != nil {
		return err
	}
	return w.appendChange(ctx, contracts.Cursor{Epoch: epoch, Sequence: change.Sequence}, change.Record, change.Op)
}
func (s *Store) ApplyBatch(ctx context.Context, b contracts.Batch) (contracts.Cursor, error) {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return contracts.Cursor{}, err
	}
	defer tx.Rollback()
	writer := newRecordWriter(tx)
	defer writer.close()
	c, err := cursorAt(ctx, tx)
	if err != nil {
		return c, err
	}
	if b.Epoch != c.Epoch || b.SubscriptionRevision != c.SubscriptionRevision {
		return c, contracts.ErrReset
	}
	if b.Through <= c.Sequence {
		return c, nil
	}
	expected := c.Sequence
	if s.pendingCapture != nil {
		expected = s.pendingCapture.Through
	}
	if b.After != expected || b.Through <= b.After {
		return c, contracts.ErrConflict
	}
	previous := b.After
	staged := s.pendingCapture != nil || b.Partial
	for _, change := range b.Changes {
		if err = validateBatchChange(change, previous, b.Through); err != nil {
			return c, err
		}
		previous = change.Sequence
		if staged {
			raw, err := json.Marshal(change)
			if err != nil {
				return c, err
			}
			if _, err = writer.exec(ctx, "INSERT INTO pending_changes(seq,payload) VALUES(?,?)", change.Sequence, raw); err != nil {
				return c, err
			}
			if err = writer.writeBlobReferences(ctx, fmt.Sprintf("capture-stage:%d", change.Sequence), change.Record); err != nil {
				return c, err
			}
		} else if err = writer.applyBatchChange(ctx, c.Epoch, change); err != nil {
			return c, err
		}
	}
	if b.Partial {
		if err = s.commitProtected(ctx, tx); err != nil {
			return c, err
		}
		if s.pendingCapture == nil {
			header := b
			header.Changes = nil
			s.pendingCapture = &header
		} else {
			s.pendingCapture.Through = b.Through
		}
		return c, nil
	}
	if staged {
		rows, err := tx.QueryContext(ctx, "SELECT payload FROM pending_changes ORDER BY seq")
		if err != nil {
			return c, err
		}
		for rows.Next() {
			var raw []byte
			var change contracts.Change
			if err = rows.Scan(&raw); err == nil {
				err = json.Unmarshal(raw, &change)
			}
			if err == nil {
				err = writer.applyBatchChange(ctx, c.Epoch, change)
			}
			if err != nil {
				rows.Close()
				return c, err
			}
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return c, err
		}
		b.After = s.pendingCapture.After
		if err = clearPendingCapture(ctx, tx); err != nil {
			return c, err
		}
	}
	if s.applyBoundary != nil && (staged || len(b.Changes) > 0) {
		s.applyBoundary()
	}
	if _, err = tx.ExecContext(ctx, "INSERT INTO applied_batches(after_seq,through_seq) VALUES(?,?)", b.After, b.Through); err != nil {
		return c, err
	}
	c.Sequence = b.Through
	if err = setCursor(ctx, tx, c); err != nil {
		return c, err
	}
	if err = s.commitProtected(ctx, tx); err != nil {
		return c, err
	}
	s.pendingCapture = nil
	s.signal()
	return c, nil
}
