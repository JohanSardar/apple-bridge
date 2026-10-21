package scenarios

import (
	"context"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runWebSourceErrors(ctx context.Context, w *world.World, client *webClient, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("source_error_authority", false)
		return
	}
	store, err := state.Open(filepath.Join(w.Dir, "source-unavailable"))
	if err != nil {
		add("source_error_state", false)
		return
	}
	if store.Close() != nil {
		add("source_error_state_closed", false)
		return
	}
	fixture := httptest.NewUnstartedServer(nil)
	fixture.Config.Handler = bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: fixture.Listener.Addr().String(), Operations: &operations.Service{Authority: authority, State: store}})
	fixture.Start()
	defer fixture.Close()
	_, port, _ := strings.Cut(fixture.Listener.Addr().String(), ":")
	unavailable := *client
	unavailable.origin = "http://localhost:" + port
	response, _, err := unavailable.request("GET", "sources", nil)
	add("source_backend_unavailable_is_503", err == nil && response.StatusCode == 503)
	req, _ := http.NewRequestWithContext(ctx, "GET", fixture.URL+"/web/v1/sources", nil)
	req.Host = "attacker.invalid"
	response, err = http.DefaultClient.Do(req)
	if err == nil {
		response.Body.Close()
	}
	add("source_forbidden_origin_is_403", err == nil && response.StatusCode == 403)
}
