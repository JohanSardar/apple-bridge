package scenarios

import (
	"context"
	"net/http/httptest"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func runIPhonePair(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-iphone-")
	add("temporary_home", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	a, err := trust.Open(dir)
	add("authority", err == nil)
	if err != nil {
		return e
	}
	server := httptest.NewTLSServer(a.IPhoneHandler())
	defer server.Close()
	code, err := a.PairIPhoneCode(server.URL, false, 10*time.Minute)
	add("one_time_code", err == nil)
	phone, err := simapple.NewIPhone()
	add("device_keys", err == nil)
	if err != nil {
		return e
	}
	status, err := phone.Pair(server.Client(), server.URL, code)
	add("http_pair", err == nil && status == 200)
	original := phone.ID
	status, err = phone.Pair(server.Client(), server.URL, code)
	add("replay_rejected", err == nil && status == 401)
	phone.ID = original
	response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/status", nil)
	add("signed_status", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	add("revoke", a.Revoke(original) == nil)
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/status", nil)
	add("revoked_key_rejected", err == nil && response.StatusCode == 401)
	if response != nil {
		response.Body.Close()
	}
	return e
}
