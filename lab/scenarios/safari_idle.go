package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os/exec"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func safariIdleReal(ctx context.Context, e Evidence) Evidence {
	return wholeHelperIdleReal(world.WithOptions(ctx, world.Options{Scope: "safari:read,system:read"}), e)
}

func wholeHelperIdleReal(ctx context.Context, e Evidence) Evidence {
	w, err := world.NewReal(ctx, "real", 0)
	e.Assertions = append(e.Assertions, Assertion{Name: "signed_helper_world", Passed: err == nil})
	if err != nil {
		e.Passed = false
		return e
	}
	defer w.Close()
	wait := func(d time.Duration) bool {
		select {
		case <-time.After(d):
			return true
		case <-ctx.Done():
			return false
		}
	}
	if !awaitIdleCapture(w) {
		e.Passed = false
		e.Assertions = append(e.Assertions, Assertion{Name: "messages_capture_and_media_settled", Passed: false})
		return e
	}
	sample := func() (int, float64, bool) {
		output, err := exec.CommandContext(ctx, "ps", "-axo", "pid=,time=,command=").Output()
		if err != nil {
			return 0, 0, false
		}
		for _, line := range strings.Split(string(output), "\n") {
			fields := strings.Fields(line)
			if len(fields) < 3 || !strings.Contains(line, w.HelperApp+"/Contents/MacOS/") || !strings.Contains(line, "session") {
				continue
			}
			pid, err := strconv.Atoi(fields[0])
			if err != nil {
				return 0, 0, false
			}
			seconds := 0.0
			for _, part := range strings.Split(fields[1], ":") {
				n, err := strconv.ParseFloat(part, 64)
				if err != nil {
					return 0, 0, false
				}
				seconds = seconds*60 + n
			}
			return pid, seconds, true
		}
		return 0, 0, false
	}
	pid, before, ok := sample()
	start := time.Now()
	if !ok || !wait(300*time.Second) {
		e.Passed = false
		e.Assertions = append(e.Assertions, Assertion{Name: "helper_cpu_sample", Passed: false})
		return e
	}
	afterPID, after, ok := sample()
	cpu := (after - before) / time.Since(start).Seconds() * 100
	passed := ok && afterPID == pid && after >= before && cpu < 2
	e.Passed = e.Passed && passed
	e.Metrics = map[string]any{"idle_seconds": time.Since(start).Seconds()}
	if ok && afterPID == pid && after >= before {
		e.Metrics["helper_cpu_percent"] = cpu
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "helper_cpu_under_two_percent_for_five_minutes", Passed: passed})
	return e
}

func awaitIdleCapture(w *world.World) bool {
	return w.Await(25*time.Minute, func() bool {
		data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "messages-idle.json"))
		var state struct {
			Idle bool `json:"idle"`
		}
		return err == nil && json.Unmarshal(data, &state) == nil && state.Idle
	}) == nil
}
