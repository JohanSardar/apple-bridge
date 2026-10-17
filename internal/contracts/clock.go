package contracts

import "time"

const FeedClockProtocol = "ntp/1"

type ClockSample struct {
	T0 time.Time `json:"t0"`
	T1 time.Time `json:"t1"`
	T2 time.Time `json:"t2"`
	T3 time.Time `json:"t3"`
}

func (s ClockSample) Estimate() (time.Duration, time.Duration, error) {
	if s.T0.IsZero() || s.T1.IsZero() || s.T2.IsZero() || s.T3.IsZero() || s.T3.Before(s.T0) || s.T2.Before(s.T1) {
		return 0, 0, ErrInvalid
	}
	rtt := s.T3.Sub(s.T0) - s.T2.Sub(s.T1)
	if rtt < 0 {
		return 0, 0, ErrInvalid
	}
	offset := (s.T1.Sub(s.T0) + s.T2.Sub(s.T3)) / 2
	return offset, rtt / 2, nil
}
