package scenarios

import (
	"context"
	"encoding/json"
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
		return runClockReal(ctx, s, e, clockpack.Pack{Runner: shortcut.CLI{}})
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
	alarm := clockpack.Alarm{ID: "synthetic-alarm", Label: "[ab-test] clock", Time: "03:17", RepeatDays: []int{}, Enabled: true}
	for _, operation := range []string{"alarms", "create_alarm", "toggle_alarm", "delete_alarm", "start_timer", "world_time"} {
		request := clockpack.Request{Operation: operation}
		readback := clockpack.Readback{Version: 1, Operation: operation, Alarms: []clockpack.Alarm{alarm}}
		switch operation {
		case "create_alarm", "toggle_alarm", "delete_alarm":
			if operation == "toggle_alarm" {
				alarm.Enabled = false
				readback.Alarms = []clockpack.Alarm{alarm}
			}
			request.Alarm = &alarm
			if operation == "delete_alarm" {
				readback.Alarms = []clockpack.Alarm{}
			}
		case "start_timer":
			request.Duration, request.Label = 90, "[ab-test] timer"
			readback.Duration, readback.Label, readback.TimerRunning = 90, request.Label, true
		case "world_time":
			request.City, readback.City, readback.LocalTime = "Chicago", "Chicago", "2026-10-28T03:17:00-05:00"
		}
		fixture.err = nil
		fixture.result.Data, _ = json.Marshal(readback)
		_, err = pack.Exchange(ctx, request)
		add("structured_"+operation, err == nil)
		var sent clockpack.Request
		add("request_"+operation, json.Unmarshal(fixture.input.Data, &sent) == nil && sent.Operation == operation)
		_, err = (clockpack.Pack{}).Exchange(ctx, request)
		add("unqualified_live_"+operation, errors.Is(err, clockpack.ErrUnqualified))
		if operation == "start_timer" {
			readback.TimerRunning = false
			fixture.result.Data, _ = json.Marshal(readback)
			_, err = pack.Exchange(ctx, request)
			add("timer_requires_running_confirmation", errors.Is(err, clockpack.ErrUnqualified))
		}
		if operation == "delete_alarm" {
			readback.Alarms = []clockpack.Alarm{alarm}
			fixture.result.Data, _ = json.Marshal(readback)
			_, err = pack.Exchange(ctx, request)
			add("delete_requires_absent_alarm", errors.Is(err, clockpack.ErrUnqualified))
		}
		fixture.result.Data = []byte(`{"version":1,"operation":"wrong"}`)
		_, err = pack.Exchange(ctx, request)
		add("mismatched_"+operation, errors.Is(err, clockpack.ErrUnqualified))
	}
	alarm.Label = "personal"
	_, err = pack.Exchange(ctx, clockpack.Request{Operation: "delete_alarm", Alarm: &alarm})
	add("untagged_alarm_rejected", err != nil)
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
