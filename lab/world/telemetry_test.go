package world

import "testing"

func TestHostQuietUsesCPUIdleNotLoad(t *testing.T) {
	idle := func(value float64) *float64 { return &value }
	cases := []struct {
		name   string
		sample HostSample
		quiet  bool
	}{
		{"disk-maintenance load with idle CPU", HostSample{Load: []float64{90, 70, 50}, CPUCount: 8, CPUIdle: idle(50)}, true},
		{"busy CPU", HostSample{Load: []float64{3, 3, 3}, CPUCount: 8, CPUIdle: idle(20)}, false},
		{"other heavy job", HostSample{Load: []float64{3, 3, 3}, CPUCount: 8, CPUIdle: idle(80), OtherHeavySlots: 1}, false},
		{"missing idle", HostSample{Load: []float64{3, 3, 3}, CPUCount: 8}, false},
		{"unavailable probe", HostSample{CPUIdle: idle(80), Unavailable: []string{"load"}}, false},
	}
	for _, c := range cases {
		if got := hostQuiet(c.sample); got != c.quiet {
			t.Errorf("%s: quiet = %v, want %v", c.name, got, c.quiet)
		}
	}
}
