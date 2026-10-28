package scenarios

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
)

func runPublicEnroll(ctx context.Context, s Scenario) Evidence {
	e := Evidence{Scenario: s.ID, Level: "web", At: time.Now().UTC(), RunID: "public-enroll", Metrics: map[string]any{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	dir, err := os.MkdirTemp("", "ab-public-enroll-")
	if err != nil {
		add("home", false)
		return e
	}
	defer os.RemoveAll(dir)
	previous, exists := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
	os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	defer func() {
		if exists {
			os.Setenv("APPLE_BRIDGE_SECRET_STORE", previous)
		} else {
			os.Unsetenv("APPLE_BRIDGE_SECRET_STORE")
		}
	}()
	authority, err := trust.Open(dir)
	if err != nil {
		add("authority", false)
		return e
	}
	local := httptest.NewUnstartedServer(nil)
	defer local.Close()
	public := httptest.NewUnstartedServer(nil)
	defer public.Close()
	_, port, _ := net.SplitHostPort(public.Listener.Addr().String())
	host := net.JoinHostPort("bridge.test", port)
	handler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, Address: local.Listener.Addr().String(), PublicHost: func() string { return host }})
	wrapper := http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/" {
			w.Header().Set("Content-Type", "text/html")
			w.Write([]byte("<!doctype html><title>Bridge Test</title>"))
			return
		}
		handler.ServeHTTP(w, r)
	})
	local.Config.Handler = wrapper
	public.Config.Handler = wrapper
	local.Start()
	public.StartTLS()
	localToken, _ := authority.CreateWebEnroll()
	publicToken, _ := authority.CreateWebEnroll()
	_, lp, _ := net.SplitHostPort(local.Listener.Addr().String())
	command := exec.CommandContext(ctx, "pnpm", "exec", "playwright", "test", "public-enroll.spec.ts", "--project=chromium-light-desktop")
	command.Dir = "web"
	report, _ := filepath.Abs("lab/evidence/private/web/public-enroll.json")
	os.MkdirAll(filepath.Dir(report), 0700)
	command.Env = append(os.Environ(), "AB_WEB_REPORT="+report, "AB_PUBLIC_URL=https://"+host, "AB_LOCAL_URL=http://localhost:"+lp, "AB_LOCAL_TOKEN="+localToken, "AB_PUBLIC_TOKEN="+publicToken)
	output, err := command.CombinedOutput()
	os.WriteFile(filepath.Join(filepath.Dir(report), "public-enroll.log"), output, 0600)
	add("public_origin_passkey_virtual_authenticator", err == nil)
	e.Passed = err == nil
	return e
}
