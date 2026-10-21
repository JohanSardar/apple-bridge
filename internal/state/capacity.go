package state

import (
	"encoding/json"
	"errors"
	"syscall"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
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
	if full || failure.Count > 0 {
		record.Data, _ = json.Marshal(data)
	}
	return record
}
