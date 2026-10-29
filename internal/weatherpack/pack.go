package weatherpack

import (
	"context"
	"encoding/json"
	"errors"
	"math"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

const Name = "AB Weather"

var ErrUnqualified = errors.New("Weather pack action identifiers and structured output are not qualified")
var ErrInvalid = errors.New("invalid weather pack request or response")

type Runner interface {
	Run(context.Context, shortcut.Input) (shortcut.Result, error)
}

type Pack struct{ Runner Runner }
type Request struct {
	Query     string  `json:"query"`
	Latitude  float64 `json:"latitude"`
	Longitude float64 `json:"longitude"`
}
type Sample struct {
	Time        time.Time `json:"time"`
	Temperature float64   `json:"temperature_celsius"`
	Condition   string    `json:"condition"`
}
type Result struct {
	Query           string          `json:"query"`
	TemperatureUnit string          `json:"temperature_unit"`
	Attribution     json.RawMessage `json:"attribution"`
	Samples         []Sample        `json:"samples"`
}

func (p Pack) Query(ctx context.Context, request Request) (Result, error) {
	var empty Result
	if request.Query != "weather.current" && request.Query != "weather.hourly" && request.Query != "weather.daily" {
		return empty, ErrUnqualified
	}
	if math.IsNaN(request.Latitude) || math.IsInf(request.Latitude, 0) || math.Abs(request.Latitude) > 90 || math.IsNaN(request.Longitude) || math.IsInf(request.Longitude, 0) || math.Abs(request.Longitude) > 180 {
		return empty, ErrInvalid
	}
	if p.Runner == nil {
		return empty, ErrUnqualified
	}
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	input, err := json.Marshal(request)
	if err != nil {
		return empty, ErrInvalid
	}
	output, err := p.Runner.Run(ctx, shortcut.Input{Shortcut: Name, Data: input, Timeout: 15 * time.Second})
	if err != nil {
		return empty, err
	}
	if ctx.Err() != nil {
		return empty, ctx.Err()
	}
	if output.TimedOut {
		return empty, context.DeadlineExceeded
	}
	if output.ExitStatus != 0 || len(output.Data) > 32<<10 {
		return empty, ErrInvalid
	}
	var result Result
	if json.Unmarshal(output.Data, &result) != nil || result.Query != request.Query || result.TemperatureUnit != "celsius" || len(result.Samples) == 0 || len(result.Samples) > 240 {
		return empty, ErrInvalid
	}
	attribution := strings.TrimSpace(string(result.Attribution))
	if attribution == "" || attribution == "null" || attribution == `""` || attribution == "{}" || attribution == "[]" {
		return empty, ErrInvalid
	}
	if request.Query == "weather.current" && len(result.Samples) != 1 {
		return empty, ErrInvalid
	}
	var fields struct {
		Samples []map[string]json.RawMessage `json:"samples"`
	}
	if json.Unmarshal(output.Data, &fields) != nil {
		return empty, ErrInvalid
	}
	for _, sample := range fields.Samples {
		if value, ok := sample["temperature_celsius"]; !ok || string(value) == "null" {
			return empty, ErrInvalid
		}
	}
	for _, sample := range result.Samples {
		if sample.Time.IsZero() || strings.TrimSpace(sample.Condition) == "" || math.IsNaN(sample.Temperature) || math.IsInf(sample.Temperature, 0) {
			return empty, ErrInvalid
		}
	}
	return result, nil
}
