package state

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Store) PendingApprovalMarkers(ctx context.Context, visibility trust.Visibility, after string) (contracts.ReadResult, error) {
	result := contracts.ReadResult{Records: []contracts.Record{}}
	sources := []string{}
	args := []any{after}
	for source, allowed := range visibility {
		if allowed {
			sources = append(sources, "?")
			args = append(args, string(source))
		}
	}
	if len(sources) == 0 {
		return result, nil
	}
	rows, err := s.db.QueryContext(ctx, "SELECT payload FROM records WHERE collection='system.command' AND deleted=0 AND json_extract(CAST(payload AS TEXT),'$.data.awaiting_approval')=1 AND id>? AND source IN ("+strings.Join(sources, ",")+") ORDER BY id LIMIT 129", args...)
	if err != nil {
		return result, err
	}
	defer rows.Close()
	for rows.Next() {
		var raw []byte
		var record contracts.Record
		if err = rows.Scan(&raw); err != nil {
			return result, err
		}
		if err = json.Unmarshal(raw, &record); err != nil {
			return result, err
		}
		if !visibility.Allows(record) {
			continue
		}
		if len(result.Records) == 128 {
			result.Next = result.Records[127].ID
			break
		}
		result.Records = append(result.Records, record)
	}
	return result, rows.Err()
}
