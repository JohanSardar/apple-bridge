package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/focuspack"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

type focusPackFixture struct {
	calls  int
	input  shortcut.Input
	result shortcut.Result
	err    error
}

func (f *focusPackFixture) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	f.calls++
	f.input = input
	return f.result, f.err
}

func runFocusPack(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	if level != "sim" {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "Set Focus identifiers and parameters are not verified; no AB Focus artifact is supplied or imported. Real set/readback/restore pending; DND syncs to Johan’s iPhone. No real effects dispatched."}
		if s.ID == "FOC-STATUS-REAL" {
			e.Conditions["reason"] = "Helper Focus authorization is not_determined; no authorization request is made."
		}
		if s.ID == "FOC-PHONE-SYNC" {
			e.Conditions["reason"] = "G12 phone access and qualified real Focus set/readback required."
		}
		if s.ID == "FOC-IDLE" {
			e.Conditions["reason"] = "Five-minute Helper CPU qualification remains pending; no idle CPU measurement claimed."
		}
		return e
	}
	add("unknown_status_has_shape", contracts.HasShape("focus.status") && len(contracts.CheckShape("focus.status", json.RawMessage(`{"focused":null,"name":null,"authorization":"not_determined"}`))) == 0)
	add("invalid_focused_rejected", len(contracts.CheckShape("focus.status", json.RawMessage(`{"focused":"false","name":null,"authorization":"authorized"}`))) > 0)
	_, mutationRegistered := contracts.Descriptor("focus.set")
	add("unqualified_mutation_not_advertised", !mutationRegistered)
	_, defaultErr := (focuspack.Pack{}).Inspect(ctx)
	add("unqualified_real_runner_disabled", errors.Is(defaultErr, focuspack.ErrUnqualified))
	fixture := &focusPackFixture{result: shortcut.Result{ExitStatus: 0, Data: []byte("synthetic")}}
	pack := focuspack.Pack{Runner: fixture}
	raw, err := pack.Inspect(ctx)
	add("fake_pack_runner", err == nil && string(raw) == "synthetic")
	add("bounded_named_pack", fixture.input.Shortcut == "AB Focus" && fixture.input.Timeout == 15*time.Second && fixture.input.Data == nil)
	for _, mode := range []string{"off", "Do Not Disturb", "Synthetic Focus"} {
		before := fixture.calls
		add("unqualified_set_never_dispatches", errors.Is(pack.Set(ctx, focuspack.SetArgs{Mode: mode, Until: "2030-01-01T00:00:00Z"}), focuspack.ErrUnqualified) && fixture.calls == before)
	}
	fixture.result.TimedOut = true
	_, err = pack.Inspect(ctx)
	add("timeout_rejected", errors.Is(err, focuspack.ErrUnqualified))
	fixture.result.TimedOut = false
	fixture.result.ExitStatus = 7
	_, err = pack.Inspect(ctx)
	add("nonzero_exit_rejected", errors.Is(err, focuspack.ErrUnqualified))
	fixture.result.ExitStatus = 0
	fixture.result.Data = make([]byte, (32<<10)+1)
	_, err = pack.Inspect(ctx)
	add("oversize_output_rejected", errors.Is(err, focuspack.ErrUnqualified))
	fixture.err = context.Canceled
	_, err = pack.Inspect(ctx)
	add("runner_error_retained", errors.Is(err, context.Canceled))

	return e
}
