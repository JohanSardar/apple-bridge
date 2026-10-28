package state

import (
	"context"
	"database/sql"
	"encoding/json"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *Store) ShapeViolations(ctx context.Context) (map[string]uint64, error) {
	out := map[string]uint64{}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return out, err
	}
	defer tx.Rollback()
	rows, err := tx.QueryContext(ctx, "SELECT payload FROM records WHERE deleted=0 AND ((collection>='messages.' AND collection<'messages/') OR (collection>='mac.' AND collection<'mac/') OR (collection>='safari.' AND collection<'safari/'))")
	if err != nil {
		return out, err
	}
	defer rows.Close()
	for rows.Next() {
		var raw []byte
		if err = rows.Scan(&raw); err != nil {
			return out, err
		}
		var r contracts.Record
		if err = json.Unmarshal(raw, &r); err != nil {
			return out, err
		}
		for _, violation := range contracts.CheckShape(r.Collection, r.Data) {
			out[violation.Key()]++
		}
	}
	return out, rows.Err()
}
