package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) SourceState(ctx context.Context, source contracts.Source, status string, coverage contracts.Coverage, capabilities []string, reason string) error {
	s.write.Lock()
	defer s.write.Unlock()
	tx, err := s.db.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer tx.Rollback()
	c, err := cursorAt(ctx, tx)
	if err != nil {
		return err
	}
	capture := contracts.Capture{Source: source, Generation: "source-state"}
	id, err := resolveID(ctx, tx, capture, contracts.Observation{Collection: "system.source", NativeKey: string(source)}, true)
	if err != nil {
		return err
	}
	old, err := recordAt(ctx, tx, "system.source", id)
	if err != nil && !errors.Is(err, sql.ErrNoRows) {
		return err
	}
	data, err := json.Marshal(map[string]any{"source": source, "state": status, "coverage": coverage, "capabilities": capabilities, "reason": reason})
	if err != nil {
		return err
	}
	if string(data) == string(old.Data) {
		return nil
	}
	r := contracts.Record{Collection: "system.source", ID: id, Version: old.Version + 1, Source: source, ObservedAt: time.Now().UTC(), Coverage: coverage, Data: data}
	c.Sequence++
	if err = writeRecord(ctx, tx, r, c.Sequence); err != nil {
		return err
	}
	if err = appendChange(ctx, tx, c, r, "upsert"); err != nil {
		return err
	}
	if err = setCursor(ctx, tx, c); err != nil {
		return err
	}
	if err = tx.Commit(); err != nil {
		return err
	}
	s.signal()
	return nil
}
