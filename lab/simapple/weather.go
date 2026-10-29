package simapple

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
	"github.com/JohanSardar/apple-bridge/internal/weatherpack"
)

type weatherFixture struct{}

func (weatherFixture) Run(context.Context, shortcut.Input) (shortcut.Result, error) {
	return shortcut.Result{Data: []byte(`{"query":"weather.current","temperature_unit":"celsius","attribution":{"provider":"synthetic"},"samples":[{"time":"2030-01-01T00:00:00Z","temperature_celsius":12,"condition":"cloudy"}]}`)}, nil
}

func syntheticWeather(ctx context.Context, raw json.RawMessage) (json.RawMessage, error) {
	var args struct {
		Latitude  *float64 `json:"latitude"`
		Longitude *float64 `json:"longitude"`
	}
	if contracts.DecodeInput(raw, &args) != nil || args.Latitude == nil || args.Longitude == nil {
		return nil, contracts.ErrInvalid
	}
	result, err := (weatherpack.Pack{Runner: weatherFixture{}}).Query(ctx, weatherpack.Request{Query: "weather.current", Latitude: *args.Latitude, Longitude: *args.Longitude})
	if err != nil {
		return nil, err
	}
	return json.Marshal(result)
}
