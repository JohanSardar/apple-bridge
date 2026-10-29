package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"github.com/JohanSardar/apple-bridge/internal/weatherpack"
)

type weatherPackFixture struct {
	calls    int
	input    shortcut.Input
	result   shortcut.Result
	err      error
	deadline bool
}

func (f *weatherPackFixture) Run(ctx context.Context, input shortcut.Input) (shortcut.Result, error) {
	f.calls++
	f.input = input
	deadline, ok := ctx.Deadline()
	f.deadline = ok && time.Until(deadline) <= 15*time.Second
	return f.result, f.err
}
func runWeatherPack(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	if level != "sim" {
		e.Status = "limited"
		e.Passed = false
		e.Conditions = map[string]string{"reason": "real qualification batched at end; Weather action identifiers and signed AB Weather pack output remain unverified"}
		return e
	}
	fixture := &weatherPackFixture{}
	pack := weatherpack.Pack{Runner: fixture}
	for _, query := range []string{"weather.current", "weather.hourly", "weather.daily"} {
		attribution := json.RawMessage(`{"provider":"synthetic","url":"https://example.invalid/weather"}`)
		raw, _ := json.Marshal(weatherpack.Result{Query: query, Attribution: attribution, Samples: []weatherpack.Sample{{Time: time.Date(2030, 1, 1, 0, 0, 0, 0, time.UTC), Temperature: 12, Condition: "synthetic"}}})
		fixture.result = shortcut.Result{ExitStatus: 0, Data: raw}
		result, err := pack.Query(ctx, weatherpack.Request{Query: query, Latitude: 40, Longitude: -90})
		add(query+"_fake_pack", err == nil && len(result.Samples) == 1 && string(result.Attribution) == string(attribution))
		var input weatherpack.Request
		add(query+"_bounded_request", json.Unmarshal(fixture.input.Data, &input) == nil && input.Query == query && input.Latitude == 40 && input.Longitude == -90 && fixture.input.Shortcut == "AB Weather" && fixture.input.Timeout == 15*time.Second && fixture.deadline)
		_, err = (weatherpack.Pack{}).Query(ctx, input)
		add(query+"_real_disabled", errors.Is(err, weatherpack.ErrUnqualified))
	}
	for _, query := range []string{"weather.current", "weather.hourly", "weather.daily", "weather.minute", "weather.alerts"} {
		_, registered := contracts.Descriptor(query)
		add(query+"_unadvertised", !registered)
	}
	for _, query := range []string{"weather.minute", "weather.alerts", "unknown"} {
		before := fixture.calls
		_, err := pack.Query(ctx, weatherpack.Request{Query: query})
		add(query+"_never_dispatched", errors.Is(err, weatherpack.ErrUnqualified) && before == fixture.calls)
	}
	request := weatherpack.Request{Query: "weather.current"}
	for _, data := range []string{`{}`, `invalid`, `{"query":"weather.current","samples":[{"time":"2030-01-01T00:00:00Z","condition":"synthetic"}]}`, `{"query":"weather.daily","attribution":"synthetic","samples":[]}`} {
		fixture.result.Data = []byte(data)
		_, err := pack.Query(ctx, request)
		add("invalid_schema_rejected", errors.Is(err, weatherpack.ErrInvalid))
	}
	fixture.result.Data = make([]byte, (32<<10)+1)
	_, err := pack.Query(ctx, request)
	add("oversize_rejected", errors.Is(err, weatherpack.ErrInvalid))
	fixture.result = shortcut.Result{TimedOut: true}
	_, err = pack.Query(ctx, request)
	add("timeout_retained", errors.Is(err, context.DeadlineExceeded))
	fixture.result = shortcut.Result{ExitStatus: 7}
	_, err = pack.Query(ctx, request)
	add("failed_exit_rejected", errors.Is(err, weatherpack.ErrInvalid))
	fixture.err = context.Canceled
	_, err = pack.Query(ctx, request)
	add("runner_error_retained", errors.Is(err, context.Canceled))
	before := fixture.calls
	_, err = pack.Query(ctx, weatherpack.Request{Query: "weather.current", Latitude: 91})
	add("invalid_location_never_dispatched", errors.Is(err, weatherpack.ErrInvalid) && before == fixture.calls)
	e.Metrics["assertion_count"] = len(e.Assertions)
	return e
}
