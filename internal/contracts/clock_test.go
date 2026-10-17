package contracts

import (
	"testing"
	"time"
)

func TestClockEstimateSeparatesOffsetAndDelay(t *testing.T) {
	base := time.Date(2026, 10, 17, 12, 0, 0, 0, time.UTC)
	for _, offset := range []time.Duration{-2 * time.Second, 0, 2 * time.Second} {
		sample := ClockSample{T0: base, T1: base.Add(2*time.Second + offset), T2: base.Add(2*time.Second + offset + time.Millisecond), T3: base.Add(4*time.Second + time.Millisecond)}
		got, uncertainty, err := sample.Estimate()
		if err != nil || got != offset || uncertainty != 2*time.Second {
			t.Fatalf("offset=%s got=%s uncertainty=%s error=%v", offset, got, uncertainty, err)
		}
	}
	asymmetric := ClockSample{T0: base, T1: base.Add(time.Second), T2: base.Add(time.Second), T3: base.Add(3 * time.Second)}
	offset, uncertainty, err := asymmetric.Estimate()
	if err != nil || offset != -500*time.Millisecond || uncertainty != 1500*time.Millisecond || offset-uncertainty > 0 || offset+uncertainty < 0 {
		t.Fatalf("asymmetric offset=%s uncertainty=%s error=%v", offset, uncertainty, err)
	}
}
