package state

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Store) ThreadWindowCounts(ctx context.Context, v trust.Visibility, window contracts.ThreadWindow) (contracts.WindowCounts, error) {
	var counts contracts.WindowCounts
	if !v[contracts.Messages] {
		return counts, contracts.ErrUnauthorized
	}
	var first, last struct {
		SentAt string `json:"sent_at"`
	}
	if json.Unmarshal(window.First.Data, &first) != nil || json.Unmarshal(window.Last.Data, &last) != nil {
		return counts, contracts.ErrInvalid
	}
	selection, args := threadSelection(window.ChatID, window.ThreadOriginatorID)
	before := append(append([]any{}, args...), modelTime(first.SentAt), window.First.ID)
	after := append(append([]any{}, args...), modelTime(last.SentAt), window.Last.ID)
	err := s.db.QueryRowContext(ctx, "SELECT (SELECT count(*) FROM "+selection+" AND (sent_at,id)<(?,?)), (SELECT count(*) FROM "+selection+" AND (sent_at,id)>(?,?))", append(before, after...)...).Scan(&counts.Older, &counts.Newer)
	return counts, err
}
