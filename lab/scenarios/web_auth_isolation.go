package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"time"
)

func runWebAuthIsolation(ctx context.Context, w *world.World, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("isolation_authority", false)
		return
	}
	var clock atomic.Int64
	clock.Store(time.Now().UnixNano())
	now := func() time.Time { return time.Unix(0, clock.Load()) }
	rateHandler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, RateClock: now})
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		r.RemoteAddr = net.JoinHostPort(r.Header.Get("X-Lab-IP"), "1234")
		rateHandler.ServeHTTP(w, r)
	}))
	defer server.Close()
	origin := strings.Replace(server.URL, "127.0.0.1", "localhost", 1)
	requests := 0
	request := func(ip, path string, input any) int {
		data, _ := json.Marshal(input)
		req, _ := http.NewRequestWithContext(ctx, "POST", origin+"/web/v1/"+path, bytes.NewReader(data))
		req.Header.Set("Origin", origin)
		req.Header.Set("X-Lab-IP", ip)
		requests++
		req.Header.Set("X-Forwarded-For", fmt.Sprintf("198.18.%d.%d", requests/256, requests%256))
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0
		}
		defer resp.Body.Close()
		io.Copy(io.Discard, resp.Body)
		return resp.StatusCode
	}
	rejected := true
	for i := 0; i < 50; i++ {
		if i > 0 {
			clock.Add(int64(61 * time.Second))
		}
		status := request("192.0.2.10", "enroll/options", map[string]string{"token": "invalid"})
		rejected = rejected && status == 401
	}
	add("ip_a_fifty_failures_rejected", rejected)
	token, err := authority.CreateWebEnroll()
	if err != nil {
		add("owner_enroll", false)
		return
	}
	owner := &webClient{ctx: ctx, origin: origin, headers: http.Header{"X-Lab-IP": []string{"192.0.2.20"}}}
	authenticator, err := newWebAuthenticator()
	if err != nil {
		add("owner_authenticator", false)
		return
	}
	status, _, err := owner.signIn(authenticator, "enroll", token, "trusted")
	add("ip_b_enrolls_immediately_after_a_fifty_failures", err == nil && status == 200)
	status, _, err = owner.signIn(authenticator, "login", "", "trusted")
	add("ip_b_signs_in_immediately_after_a_fifty_failures", err == nil && status == 200)
	add("only_failing_ip_is_backed_off", request("192.0.2.10", "login/options", map[string]any{}) == 429)
	clock.Add(int64(42 * time.Second))
	add("failing_ip_backoff_expires", request("192.0.2.10", "login/options", map[string]any{}) == 200)

	for i := 0; i < 10; i++ {
		request(fmt.Sprintf("198.51.100.%d", i+1), "enroll/options", map[string]string{"token": "invalid"})
	}
	add("distributed_failures_do_not_lock_owner", request("203.0.113.10", "login/options", map[string]any{}) == 200)
	clock.Add(int64(time.Minute))
	ceiling := true
	for i := 0; i < 600; i++ {
		ceiling = ceiling && request(fmt.Sprintf("198.19.%d.%d", i/256, i%256), "login/options", map[string]any{}) == 200
	}
	add("global_ceiling_allows_600_auth_requests", ceiling)
	add("global_ceiling_rejects_request_601", request("198.19.3.1", "login/options", map[string]any{}) == 429)
	clock.Add(int64(time.Minute))
	add("global_resource_ceiling_resets_next_minute", request("203.0.113.20", "login/options", map[string]any{}) == 200)

}
