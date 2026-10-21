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
	if json.Unmarshal(window.First.Data, &first) != nil || json.Unmarshal(window.Last.Data, &last) != nil || first.SentAt == "" || last.SentAt == "" {
		return counts, contracts.ErrInvalid
	}
	err := s.db.QueryRowContext(ctx, `SELECT
 (SELECT count(*) FROM msg_by_chat WHERE chat_id=? AND (sent_at,id)<(?,?)),
 (SELECT count(*) FROM msg_by_chat WHERE chat_id=? AND (sent_at,id)>(?,?))`, window.ChatID, modelTime(first.SentAt), window.First.ID, window.ChatID, modelTime(last.SentAt), window.Last.ID).Scan(&counts.Older, &counts.Newer)
	return counts, err
}
