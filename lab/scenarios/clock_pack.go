package scenarios

import (
	"context"
	"errors"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/clockpack"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type clockPackFixture struct {
	input  shortcut.Input
	result shortcut.Result
	err    error
}

func (f *clockPackFixture) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	f.input = input
	return f.result, f.err
}

func runClockPack(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	if level != "sim" {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "Leader must import AB Clock; structured alarm fields and timer running confirmation are unqualified. No real effects dispatched."}
		return e
	}
	fixture := &clockPackFixture{result: shortcut.Result{ExitStatus: 0, Data: []byte("synthetic")}}
	pack := clockpack.Pack{Runner: fixture}
	raw, err := pack.Inspect(ctx)
	add("fake_pack_runner", err == nil && string(raw) == "synthetic")
	add("bounded_named_pack", fixture.input.Shortcut == "AB Clock" && fixture.input.Timeout == 15*time.Second && fixture.input.Data == nil)
	add("completion_never_establishes_alarm_capture", errors.Is(pack.Alarms(ctx), clockpack.ErrUnqualified))
	fixture.result.TimedOut = true
	_, err = pack.Inspect(ctx)
	add("timeout_rejected", errors.Is(err, clockpack.ErrUnqualified))
	fixture.result.TimedOut = false
	fixture.result.ExitStatus = 7
	_, err = pack.Inspect(ctx)
	add("nonzero_exit_rejected", errors.Is(err, clockpack.ErrUnqualified))
	fixture.result.ExitStatus = 0
	fixture.result.Data = make([]byte, (32<<10)+1)
	_, err = pack.Inspect(ctx)
	add("oversize_output_rejected", errors.Is(err, clockpack.ErrUnqualified))
	fixture.err = context.Canceled
	_, err = pack.Inspect(ctx)
	add("runner_error_retained", errors.Is(err, context.Canceled))
	dir, err := os.MkdirTemp("", "ab-clock-build-")
	add("temporary_build", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	command := exec.CommandContext(ctx, "go", "run", "./tools/shortcuts", "build", "-source", "packaging/shortcuts/ab-clock.json", "-out", dir, "-unsigned")
	output, err := command.CombinedOutput()
	add("repo_source_compiles", err == nil)
	if err != nil {
		os.Stderr.Write(output)
		return e
	}
	_, err = os.Stat(filepath.Join(dir, "ab-clock.unsigned.shortcut"))
	add("unsigned_artifact", err == nil)
	for _, op := range []string{"clock.create_alarm", "clock.toggle_alarm", "clock.delete_alarm", "clock.start_timer", "clock.world_time"} {
		_, registered := contracts.Descriptor(op)
		add("unqualified_operation_not_advertised", !registered)
	}
	return e
}
