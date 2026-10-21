package scenarios

import (
	"context"
	"net/http"
	"net/http/httptest"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webCookieBoundaries(ctx context.Context, w *world.World, token string) (map[string]int, error) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		return nil, err
	}
	publicHost := "bridge.example.com:10000"
	fixture := httptest.NewUnstartedServer(nil)
	fixture.Config.Handler = bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: fixture.Listener.Addr().String(), PublicHost: func() string { return publicHost }})
	fixture.StartTLS()
	defer fixture.Close()
	checks := map[string]int{}
	for _, check := range []struct{ name, endpoint, host, cookie string }{
		{"public_local", fixture.URL, publicHost, "ab_session_local"},
		{"tls_local", fixture.URL, fixture.Listener.Addr().String(), "ab_session_local"},
		{"http_host", w.Gateway.URL, w.Gateway.Host, bridgeweb.SessionCookie},
		{"http_local", w.Gateway.URL, w.Gateway.Host, "ab_session_local"},
		{"public_host", fixture.URL, publicHost, bridgeweb.SessionCookie},
	} {
		request, err := http.NewRequestWithContext(ctx, "GET", check.endpoint+"/web/v1/session", nil)
		if err != nil {
			return nil, err
		}
		request.Host = check.host
		request.AddCookie(&http.Cookie{Name: check.cookie, Value: token})
		response, err := fixture.Client().Do(request)
		if err != nil {
			return nil, err
		}
		checks[check.name] = response.StatusCode
		response.Body.Close()
	}
	return checks, nil
}
