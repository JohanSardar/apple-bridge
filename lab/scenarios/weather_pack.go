package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"github.com/JohanSardar/apple-bridge/internal/weatherpack"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
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
		return runWeatherReal(ctx, s, level)
	}
	fixture := &weatherPackFixture{}
	pack := weatherpack.Pack{Runner: fixture}
	for _, query := range []string{"weather.current", "weather.hourly", "weather.daily"} {
		attribution := json.RawMessage(`{"provider":"synthetic","url":"https://example.invalid/weather"}`)
		raw, _ := json.Marshal(weatherpack.Result{Query: query, TemperatureUnit: "celsius", Attribution: attribution, Samples: []weatherpack.Sample{{Time: time.Date(2030, 1, 1, 0, 0, 0, 0, time.UTC), Temperature: 12, Condition: "synthetic"}}})
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
		add(query+"_registration", registered == (query == "weather.current" || query == "weather.hourly" || query == "weather.daily"))
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

func runWeatherReal(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	w, err := world.NewReal(ctx, level, 0)
	add("world_started", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "weather:read,system:read")
	add("scopes_granted", err == nil)
	if err != nil {
		return e
	}
	client := scripted.Client{Endpoint: endpoint}
	err = w.Await(20*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &out) == nil
	})
	add("helper_ready", err == nil)
	if err != nil {
		return e
	}
	for _, query := range []string{"weather.current", "weather.hourly", "weather.daily"} {
		var out contracts.QueryResult
		err = client.Call(ctx, query, contracts.Query{Args: json.RawMessage(`{"latitude":40,"longitude":-90}`), DeadlineMS: 15000}, &out)
		if err != nil {
			e.Status = "limited"
			e.Passed = false
			e.Conditions = map[string]string{"reason": "needs WeatherKit-enabled provisioning profile and Apple-signed Mac Helper"}
			return e
		}
		fixture := &weatherPackFixture{result: shortcut.Result{Data: out.ResultJSON}}
		result, decodeErr := (weatherpack.Pack{Runner: fixture}).Query(ctx, weatherpack.Request{Query: query, Latitude: 40, Longitude: -90})
		add(query+"_structured_units_attribution", decodeErr == nil)
		if decodeErr != nil {
			return e
		}
		valid := len(result.Samples) > 0
		if query == "weather.hourly" {
			valid = len(result.Samples) >= 24
		}
		if query == "weather.daily" {
			valid = len(result.Samples) >= 5
		}
		for i, sample := range result.Samples {
			valid = valid && sample.Temperature >= -100 && sample.Temperature <= 65 && sample.Time.After(time.Now().Add(-24*time.Hour)) && sample.Time.Before(time.Now().Add(11*24*time.Hour))
			if i > 0 {
				valid = valid && sample.Time.After(result.Samples[i-1].Time)
			}
		}
		add(query+"_plausible_forecast", valid)
		e.Metrics[query+"_sample_count"] = len(result.Samples)
	}
	return e
}
