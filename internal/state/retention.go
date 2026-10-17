package state

import (
	"context"
	"database/sql"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type snapshotLease struct {
	Sequence  uint64
	ExpiresAt time.Time
}

type Retention struct {
	Rows uint64
	Age  time.Duration
}

func (s *Store) Prune(ctx context.Context, policy Retention) error {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	head, err := cursorAt(ctx, tx)
	if err != nil {
		return err
	}
	if head.Sequence <= policy.Rows {
		return nil
	}
	floor := head.Sequence - policy.Rows
	var old sql.NullInt64
	if err = tx.QueryRowContext(ctx, "SELECT MAX(seq) FROM journal WHERE committed_at < ?", time.Now().Add(-policy.Age).Unix()).Scan(&old); err != nil {
		return err
	}
	if !old.Valid {
		return nil
	}
	floor = min(floor, uint64(old.Int64))
	now := time.Now()
	for id, lease := range s.leases {
		if now.After(lease.ExpiresAt) {
			delete(s.leases, id)
		} else {
			floor = min(floor, lease.Sequence)
		}
	}
	var current uint64
	if err = tx.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='retention_floor'").Scan(&current); err != nil {
		return err
	}
	if floor <= current {
		return nil
	}
	if _, err = tx.ExecContext(ctx, "DELETE FROM journal WHERE seq<=?", floor); err != nil {
		return err
	}
	if _, err = tx.ExecContext(ctx, "UPDATE meta SET value=? WHERE key='retention_floor'", fmt.Sprint(floor)); err != nil {
		return err
	}
	return tx.Commit()
}

func (f *FrozenSnapshot) Renew(ctx context.Context) error {
	f.store.write.Lock()
	defer f.store.write.Unlock()
	lease, ok := f.store.leases[f.ID]
	if !ok || time.Now().After(lease.ExpiresAt) {
		return contracts.ErrExpired
	}
	lease.ExpiresAt = time.Now().Add(10 * time.Minute)
	f.store.leases[f.ID] = lease
	return ctx.Err()
}

func (s *Store) Restrict(ctx context.Context, visibility trust.Visibility) error {
	s.write.Lock()
	defer s.write.Unlock()
	args := []any{}
	placeholders := []string{}
	for source, allowed := range visibility {
		if allowed {
			placeholders = append(placeholders, "?")
			args = append(args, string(source))
		}
	}
	if len(args) == 0 {
		placeholders = append(placeholders, "?")
		args = append(args, "")
	}
	_, err := s.db.ExecContext(ctx, "DELETE FROM records WHERE source NOT IN ("+strings.Join(placeholders, ",")+")", args...)
	if err == nil {
		s.signal()
	}
	return err
}
