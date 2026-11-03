package scenarios

import (
	"encoding/json"
	"time"
)

type safetyEvent struct {
	Event      string   `json:"event" yaml:"event"`
	Fence      uint64   `json:"fence" yaml:"fence"`
	Checkpoint string   `json:"checkpoint" yaml:"checkpoint"`
	Allowed    []string `json:"allowed" yaml:"allowed"`
	Sources    []string `json:"sources" yaml:"sources"`
}

type faultTraceOptions struct {
	Fault   string        `json:"fault" yaml:"fault"`
	Healthy []safetyEvent `json:"healthy" yaml:"healthy"`
	Mutant  []safetyEvent `json:"mutant" yaml:"mutant"`
}

func observeSafety(events []safetyEvent) string {
	fence := uint64(0)
	commits := map[string]bool{}
	latest := ""
	for _, event := range events {
		switch event.Event {
		case "owner":
			if event.Fence <= fence {
				return "invalid-owner-sequence"
			}
			fence = event.Fence
		case "accepted":
			if fence == 0 || event.Fence != fence {
				return "accept-stale-fence"
			}
		case "commit":
			if event.Checkpoint == "" || commits[event.Checkpoint] {
				return "invalid-commit-sequence"
			}
			commits[event.Checkpoint] = true
			latest = event.Checkpoint
		case "ack":
			if !commits[event.Checkpoint] {
				return "ack-uncommitted-data"
			}
		case "reopen":
			if latest == "" || latest != event.Checkpoint {
				return "skip-checkpoint-commit"
			}
		case "read":
			allowed := map[string]bool{}
			for _, source := range event.Allowed {
				allowed[source] = true
			}
			for _, source := range event.Sources {
				if !allowed[source] {
					return "leak-scope"
				}
			}
		default:
			return "invalid-event"
		}
	}
	return ""
}

func runSeededFaults(s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Conditions: map[string]string{"gate_kind": "protocol_trace"}}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	add("sim_level", level == "sim")
	count := 0
	for _, step := range s.When {
		input, ok := step["fault_trace"]
		if !ok || len(step) != 1 {
			add("valid_trace_step", false)
			continue
		}
		raw, err := json.Marshal(input)
		var gate struct {
			Fault   string        `json:"fault"`
			Healthy []safetyEvent `json:"healthy"`
			Mutant  []safetyEvent `json:"mutant"`
		}
		if err != nil || json.Unmarshal(raw, &gate) != nil || len(gate.Healthy) == 0 || len(gate.Mutant) == 0 {
			add("valid_trace_input", false)
			continue
		}
		valid := gate.Fault == "skip-checkpoint-commit" || gate.Fault == "accept-stale-fence" || gate.Fault == "ack-uncommitted-data" || gate.Fault == "leak-scope"
		add("known_fault", valid)
		add("healthy_trace", observeSafety(gate.Healthy) == "")
		add(gate.Fault, valid && observeSafety(gate.Mutant) == gate.Fault)
		count++
	}
	add("trace_present", count > 0)
	return e
}
