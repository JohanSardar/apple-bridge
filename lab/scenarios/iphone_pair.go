package scenarios

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/phonefixture"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
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
	selection := world.ImplementationSelection(ctx)
	goBinary, err := filepath.Abs("bin/bridge")
	if err != nil {
		add("authority", false)
		return e
	}
	binary, err := selection.Binary(selection.Gateway, goBinary)
	if err != nil {
		add("authority", false)
		return e
	}
	gateway, err := phonefixture.Start(ctx, binary, filepath.Join(dir, "home"))
	add("authority", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
		return e
	}
	defer gateway.Close()
	output, err := gateway.Command("pair", "--iphone", "--via-remote")
	code := strings.Split(output, "\n")[0]
	add("one_time_code", err == nil)
	phone, err := simapple.NewIPhone()
	add("device_keys", err == nil)
	if err != nil {
		return e
	}
	status, err := phone.Pair(gateway.Client, gateway.URL, code)
	add("http_pair", err == nil && status == 200)
	original := phone.ID
	status, err = phone.Pair(gateway.Client, gateway.URL, code)
	add("replay_rejected", err == nil && status == 401)
	phone.ID = original
	response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/status", nil)
	add("signed_status", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	_, err = gateway.Command("revoke", original)
	add("revoke", err == nil)
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/status", nil)
	add("revoked_key_rejected", err == nil && response.StatusCode == 401)
	if response != nil {
		response.Body.Close()
	}
	return e
}
