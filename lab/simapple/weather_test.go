package simapple

import (
	"context"
	"encoding/json"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/weatherpack"
)

func TestSyntheticWeather(t *testing.T) {
	raw, err := syntheticWeather(context.Background(), json.RawMessage(`{"latitude":37.7955,"longitude":-122.3937}`))
	var result weatherpack.Result
	if err != nil || json.Unmarshal(raw, &result) != nil || len(result.Samples) != 1 || result.Samples[0].Temperature != 12 || len(result.Attribution) == 0 {
		t.Fatal("synthetic weather failed")
	}
	for _, invalid := range []string{`{}`, `{"latitude":91,"longitude":0}`, `{"latitude":0,"longitude":0,"extra":1}`} {
		if _, err := syntheticWeather(context.Background(), json.RawMessage(invalid)); err == nil {
			t.Fatal("invalid request accepted")
		}
	}
}
