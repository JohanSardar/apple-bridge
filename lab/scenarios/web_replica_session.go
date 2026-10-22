package scenarios

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webReplicaSession(ctx context.Context, w *world.World, cached **world.Endpoint, waitForCapture bool) (map[string]any, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if *cached == nil {
		if err := w.AddReplicaScope("messages:write,system:read"); err != nil {
			return nil, err
		}
		endpoint := w.Replicas[len(w.Replicas)-1]
		*cached = &endpoint
	}
	endpoint := **cached
	if waitForCapture {
		target, err := w.Cursor(w.Gateway)
		if err != nil {
			return nil, err
		}
		err = w.Await(20*time.Second, func() bool {
			cursor, err := w.Cursor(endpoint)
			if err != nil || cursor.Epoch != target.Epoch || cursor.Sequence < target.Sequence {
				return false
			}
			status, err := w.Status(endpoint)
			return err == nil && status.State == "connected"
		})
		if err != nil {
			return nil, err
		}
	}
	authority, err := trust.Open(endpoint.Home)
	if err != nil {
		return nil, err
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	origin := strings.Replace(endpoint.URL, "127.0.0.1", "localhost", 1)
	return map[string]any{"url": origin, "cookie": map[string]any{"name": bridgeweb.LocalSessionCookie, "value": token, "url": origin, "httpOnly": true, "secure": false, "sameSite": "Strict"}}, nil
}

func webLookupRate(ctx context.Context, w *world.World, cached **world.Endpoint) (map[string]any, error) {
	if _, err := webReplicaSession(ctx, w, cached, false); err != nil {
		return nil, err
	}
	authority, err := trust.Open((**cached).Home)
	if err != nil {
		return nil, err
	}
	var clock atomic.Int64
	clock.Store(time.Now().UnixNano())
	fixture := httptest.NewServer(bridgeweb.Handler(bridgeweb.Deps{Authority: authority, RateClock: func() time.Time { return time.Unix(0, clock.Load()) }}))
	defer fixture.Close()
	client := &webClient{ctx: ctx, origin: strings.Replace(fixture.URL, "127.0.0.1", "localhost", 1)}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	client.cookie = &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}
	statuses := []int{}
	retryAfter := ""
	for index := 0; index < 21; index++ {
		response, _, problem := client.request("POST", "lookup", map[string]any{"q": "Demo"})
		if problem != nil {
			return nil, problem
		}
		statuses = append(statuses, response.StatusCode)
		if response.StatusCode == 429 {
			retryAfter = response.Header.Get("Retry-After")
		}
	}
	token, _, err = authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	first := client.cookie
	client.cookie = &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}
	isolated, _, err := client.request("POST", "lookup", map[string]any{"q": "Demo"})
	if err != nil {
		return nil, err
	}
	client.cookie = first
	clock.Add(int64(time.Second))
	reset, _, err := client.request("POST", "lookup", map[string]any{"q": "Demo"})
	if err != nil {
		return nil, err
	}
	return map[string]any{"statuses": statuses, "retry_after": retryAfter, "after_reset": reset.StatusCode, "isolated_session": isolated.StatusCode}, nil
}
