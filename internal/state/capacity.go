package state

import (
	"context"
	"encoding/json"
	"errors"
	"syscall"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func IsCapacity(err error) bool {
	var sqlite interface{ Code() int }
	return errors.Is(err, syscall.ENOSPC) || (errors.As(err, &sqlite) && sqlite.Code()&255 == 13)
}
func (s *Store) Capacity(source contracts.Source, err error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if IsCapacity(err) {
		s.capacity[source] = true
	} else if err == nil {
		clear(s.capacity)
	}
}
func (s *Store) sourceStatus(record contracts.Record) contracts.Record {
	if record.Collection != "system.source" {
		return record
	}
	var data map[string]any
	if json.Unmarshal(record.Data, &data) != nil {
		return record
	}
	source, _ := data["source"].(string)
	s.mu.Lock()
	full := s.capacity[contracts.Source(source)]
	failure := s.captureFailures[contracts.Source(source)]
	s.mu.Unlock()
	if failure.Count > 0 {
		reason := "gateway_capture_rejected:" + failure.Code
		data["capture_rejected_count"] = failure.Count
		data["last_capture_rejected_reason"] = reason
		if failure.Active {
			data["state"] = "degraded"
			data["reason"] = reason
		}
	}
	if full {
		data["state"] = "degraded"
		data["reason"] = "capacity"
	}
	if group, ok := contracts.GrantsForSource(contracts.Source(source)); ok {
		data["grants"] = group.Grants
	}
	record.Data, _ = json.Marshal(data)
	return record
}

func (s *Store) SourceStatuses(ctx context.Context, visibility trust.Visibility) (contracts.ReadResult, error) {
	result, err := s.Read(ctx, visibility, contracts.ReadQuery{Collection: "system.source"})
	if err != nil {
		return result, err
	}
	for i, record := range result.Records {
		result.Records[i] = s.sourceStatus(record)
	}
	return result, nil
}
