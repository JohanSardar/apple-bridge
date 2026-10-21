package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing/fstest"
)

func runWebPinnedHosts(ctx context.Context, w *world.World, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("security_authority", false)
		return
	}
	fixture := httptest.NewUnstartedServer(nil)
	handler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: fixture.Listener.Addr().String(), Assets: fstest.MapFS{}})
	fixture.Config.Handler = http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		handler.ServeHTTP(w, r)
	})
	fixture.StartTLS()
	defer fixture.Close()
	tlsRequest := func(host, path string) int {
		req, _ := http.NewRequestWithContext(ctx, "GET", fixture.URL+path, nil)
		req.Host = host
		resp, err := fixture.Client().Do(req)
		if err != nil {
			return 0
		}
		defer resp.Body.Close()
		return resp.StatusCode
	}
	add("tls_arbitrary_host_rejected", tlsRequest("attacker.invalid", "/") == 403)
	add("tls_localhost_allowed", tlsRequest(fixture.Listener.Addr().String(), "/") == 200)
	fixture2 := httptest.NewUnstartedServer(nil)
	fixture2.Config.Handler = bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: fixture2.Listener.Addr().String(), Assets: fstest.MapFS{}})
	fixture2.Start()
	defer fixture2.Close()
	req, _ := http.NewRequestWithContext(ctx, "GET", fixture2.URL+"/", nil)
	req.Host = "attacker.invalid"
	resp, err := http.DefaultClient.Do(req)
	if err == nil {
		resp.Body.Close()
	}
	add("http_arbitrary_host_rejected", err == nil && resp.StatusCode == 403)

	publicHost := "bridge.example.invalid:10000"
	securePublic := httptest.NewUnstartedServer(nil)
	securePublic.Config.Handler = bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: securePublic.Listener.Addr().String(), PublicHost: func() string { return publicHost }, Assets: fstest.MapFS{}})
	securePublic.StartTLS()
	defer securePublic.Close()
	publicRequest, _ := http.NewRequestWithContext(ctx, "POST", securePublic.URL+"/web/v1/login/options", strings.NewReader("{}"))
	publicRequest.Host = publicHost
	publicRequest.Header.Set("Origin", "https://"+publicHost)
	publicResponse, publicErr := securePublic.Client().Do(publicRequest)
	var options struct {
		PublicKey struct {
			RPID string `json:"rpId"`
		} `json:"publicKey"`
	}
	if publicErr == nil {
		json.NewDecoder(publicResponse.Body).Decode(&options)
		publicResponse.Body.Close()
	}
	add("tls_pinned_public_origin_and_rpid", publicErr == nil && publicResponse.StatusCode == 200 && options.PublicKey.RPID == "bridge.example.invalid")
	publicRequest, _ = http.NewRequestWithContext(ctx, "GET", securePublic.URL+"/", nil)
	publicRequest.Host = "bridge.example.invalid:8443"
	publicResponse, publicErr = securePublic.Client().Do(publicRequest)
	if publicErr == nil {
		publicResponse.Body.Close()
	}
	add("tls_unconfigured_public_port_rejected", publicErr == nil && publicResponse.StatusCode == 403)

}
