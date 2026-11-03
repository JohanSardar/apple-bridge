package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"time"
)

func runWebRateWindows(ctx context.Context, w *world.World, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("rate_authority", false)
		return
	}
	var clock atomic.Int64
	clock.Store(time.Now().UnixNano())
	now := func() time.Time { return time.Unix(0, clock.Load()) }
	fixture := httptest.NewServer(bridgeweb.Handler(bridgeweb.Deps{Authority: authority, RateClock: now}))
	defer fixture.Close()
	c := &webClient{ctx: ctx, origin: strings.Replace(fixture.URL, "127.0.0.1", "localhost", 1)}
	daily := true
	for minute := 0; minute < 10; minute++ {
		for count := 0; count < 10; count++ {
			resp, _, err := c.request("POST", "login/options", map[string]any{})
			daily = daily && err == nil && resp.StatusCode == 200
		}
		clock.Add(int64(time.Minute))
	}
	resp, _, err := c.request("POST", "login/options", map[string]any{})
	add("signin_daily_100_limit", daily && err == nil && resp.StatusCode == 429)
	clock.Add(int64(24 * time.Hour))
	resp, _, err = c.request("POST", "login/options", map[string]any{})
	add("signin_next_day_reset", err == nil && resp.StatusCode == 200)
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		add("api_session_fixture", false)
		return
	}
	c.cookie = &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}
	api := true
	for count := 0; count < 50; count++ {
		resp, _, err = c.request("GET", "session", nil)
		api = api && err == nil && resp.StatusCode == 200
	}
	resp, _, err = c.request("GET", "session", nil)
	add("api_50_per_second", api && err == nil && resp.StatusCode == 429)
	clock.Add(int64(time.Second))
	resp, _, err = c.request("GET", "session", nil)
	add("api_second_reset", err == nil && resp.StatusCode == 200)
	handler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, RateClock: now})
	distributed := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		r.RemoteAddr = net.JoinHostPort(r.Header.Get("X-Lab-IP"), "1234")
		handler.ServeHTTP(w, r)
	}))
	defer distributed.Close()
	failed := true
	requests := 0
	request := func(ip, path string, input any) int {
		raw, _ := json.Marshal(input)
		origin := strings.Replace(distributed.URL, "127.0.0.1", "localhost", 1)
		req, _ := http.NewRequestWithContext(ctx, "POST", origin+"/web/v1/"+path, bytes.NewReader(raw))
		req.Header.Set("Origin", origin)
		req.Header.Set("X-Lab-IP", ip)
		requests++
		req.Header.Set("X-Forwarded-For", fmt.Sprintf("198.18.%d.%d", requests/256, requests%256))
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0
		}
		io.Copy(io.Discard, resp.Body)
		resp.Body.Close()
		return resp.StatusCode
	}
	for i := 0; i < 10; i++ {
		failed = failed && request(fmt.Sprintf("192.0.2.%d", i+1), "enroll/options", map[string]any{"token": "invalid"}) == 401
	}
	add("failure_backoff_isolated_across_clients", failed && request("192.0.2.99", "login/options", map[string]any{}) == 200)
	clock.Add(int64(2 * time.Second))

	add("unaffected_client_stays_available", request("192.0.2.99", "login/options", map[string]any{}) == 200)
	shared := true
	for i := 0; i < 10; i++ {
		shared = shared && request(fmt.Sprintf("2001:db8:1::%x", i+1), "login/options", map[string]any{}) == 200
	}
	add("ipv6_64_shared_rate_bucket", shared && request("2001:db8:1::ffff", "login/options", map[string]any{}) == 429)
	add("ipv6_distinct_64_independent", request("2001:db8:2::1", "login/options", map[string]any{}) == 200)
	sameSource := true
	for i := 0; i < 10; i++ {
		sameSource = sameSource && request("192.0.2.30", "login/options", map[string]any{}) == 200
	}
	add("local_forwarded_source_header_does_not_evade_limit", sameSource && request("192.0.2.30", "login/options", map[string]any{}) == 429)

	live := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: c.cookie}
	socket, err := live.socket()
	if err != nil {
		add("websocket_rate_fixture", false)
		return
	}
	defer socket.CloseNow()
	for i := 0; i < 51; i++ {
		if socket.Write(ctx, websocket.MessageText, []byte("[")) != nil {
			add("websocket_rate_send", false)
			return
		}
	}
	limited := false
	bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
	defer cancel()
	for i := 0; i < 60; i++ {
		var frame map[string]any
		if wsjson.Read(bounded, socket, &frame) != nil {
			break
		}
		if frame["code"] == "rate_limited" {
			limited = true
			break
		}
	}
	add("malformed_live_frames_count_toward_rate_limit", limited)
}
