package state

import "github.com/JohanSardar/apple-bridge/internal/contracts"

type captureFailure struct {
	Count  uint64
	Code   string
	Active bool
}

func (s *Store) CaptureResult(source contracts.Source, code string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.captureFailures == nil {
		s.captureFailures = map[contracts.Source]captureFailure{}
	}
	failure := s.captureFailures[source]
	failure.Active = code != ""
	if failure.Active {
		failure.Count++
		failure.Code = code
	}
	s.captureFailures[source] = failure
}
