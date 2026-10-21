package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"net/url"
	"os/exec"
	"strings"
	"time"
)

func runAgentAccess(ctx context.Context, s Scenario, level string, run func(context.Context, *world.World, *webClient, *Evidence, func(string, bool))) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, "sim", 1, 1, world.BlobOptions{Scope: "admin"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	a, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("authority", false)
		return e
	}
	token, _, err := a.CreateDeviceSession("trusted")
	if err != nil {
		add("owner_session", false)
		return e
	}
	c := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
	resp, _, err := c.request("GET", "agents", nil)
	add("agents_available", err == nil && resp.StatusCode == 200)
	run(ctx, w, c, &e, add)
	e.Passed = true
	for _, v := range e.Assertions {
		e.Passed = e.Passed && v.Passed
	}
	return e
}

func agentRequest(ctx context.Context, base, token, method, path string, input any) (int, []byte, http.Header) {
	var body io.Reader
	if values, ok := input.(url.Values); ok {
		body = strings.NewReader(values.Encode())
	} else if input != nil {
		raw, _ := json.Marshal(input)
		body = bytes.NewReader(raw)
	}
	req, _ := http.NewRequestWithContext(ctx, method, base+path, body)
	if token != "" {
		req.Header.Set("Authorization", "Bearer "+token)
	}
	req.Header.Set("Accept", "application/json, text/event-stream")
	req.Header.Set("Content-Type", "application/json")
	if _, ok := input.(url.Values); ok {
		req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	}
	client := &http.Client{Timeout: 10 * time.Second, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	resp, err := client.Do(req)
	if err != nil {
		return 0, nil, nil
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
	return resp.StatusCode, raw, resp.Header
}
