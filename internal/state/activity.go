package state

import (
	"context"
	"database/sql"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"time"
)

type AgentActivity struct {
	Sequence      uint64    `json:"-"`
	ID            string    `json:"id"`
	Principal     string    `json:"principal"`
	Operation     string    `json:"operation"`
	Kind          string    `json:"kind"`
	State         string    `json:"state"`
	At            time.Time `json:"at"`
	Count         int       `json:"count"`
	ArgumentBytes int       `json:"argument_bytes"`
	ArgumentHash  string    `json:"argument_hash"`
}

func (s *Store) RecordAgentActivity(ctx context.Context, a AgentActivity) error {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	if err = writeAgentActivity(ctx, tx, a); err != nil {
		return err
	}
	return tx.Commit()
}
func (s *Store) AgentActivity(ctx context.Context, principal, operation string) ([]AgentActivity, error) {
	if err := s.PruneAgentActivity(ctx); err != nil {
		return nil, err
	}
	rows, err := s.db.QueryContext(ctx, `SELECT id,principal,operation,kind,state,at,count,argument_bytes,argument_hash FROM agent_activity WHERE (?='' OR principal=?) AND (?='' OR operation=?) ORDER BY at DESC,id DESC LIMIT 1000`, principal, principal, operation, operation)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	out := []AgentActivity{}
	for rows.Next() {
		var a AgentActivity
		var at int64
		if err = rows.Scan(&a.ID, &a.Principal, &a.Operation, &a.Kind, &a.State, &at, &a.Count, &a.ArgumentBytes, &a.ArgumentHash); err != nil {
			return nil, err
		}
		a.At = time.Unix(at, 0).UTC()
		out = append(out, a)
	}
	return out, rows.Err()
}
func RedactedArguments(args []byte) map[string]any {
	return map[string]any{"argument_bytes": len(args), "argument_hash": trust.Fingerprint(args)}
}

func (s *Store) PruneAgentActivity(ctx context.Context) error {
	s.write.Lock()
	defer s.write.Unlock()
	_, err := s.db.ExecContext(ctx, "DELETE FROM agent_activity WHERE at < ?", time.Now().Add(-90*24*time.Hour).Unix())
	return err
}

func writeAgentActivity(ctx context.Context, tx *sql.Tx, a AgentActivity) error {
	if _, err := tx.ExecContext(ctx, "DELETE FROM agent_activity WHERE at < ?", time.Now().Add(-90*24*time.Hour).Unix()); err != nil {
		return err
	}
	increment := 0
	if a.Kind == "read" {
		increment = 1
	}
	_, err := tx.ExecContext(ctx, `INSERT INTO agent_activity(id,principal,operation,kind,state,at,count,argument_bytes,argument_hash,sequence) VALUES(?,?,?,?,?,?,1,?,?,?) ON CONFLICT(id) DO UPDATE SET state=excluded.state,count=agent_activity.count+?,argument_bytes=excluded.argument_bytes,argument_hash=excluded.argument_hash,sequence=excluded.sequence WHERE agent_activity.kind='read' OR excluded.sequence>=agent_activity.sequence`, a.ID, a.Principal, a.Operation, a.Kind, a.State, a.At.Unix(), a.ArgumentBytes, a.ArgumentHash, a.Sequence, increment)
	if err != nil {
		return err
	}
	return nil
}
