package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"net/url"
)

func demoSessionCheck(ctx context.Context, server *demo.Server, w *world.World, add func(string, bool)) {
	client := &http.Client{CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	response, err := client.Get(server.URL)
	add("demo_link_redirects_to_app", err == nil && response.StatusCode == 303 && response.Header.Get("Location") == "/messages")
	if err != nil {
		return
	}
	io.Copy(io.Discard, response.Body)
	response.Body.Close()
	var cookie *http.Cookie
	for _, value := range response.Cookies() {
		if value.Name == demo.SessionCookie {
			cookie = value
		}
	}
	add("demo_session_cookie", cookie != nil && cookie.HttpOnly && !cookie.Secure && cookie.SameSite == http.SameSiteStrictMode)
	if cookie == nil {
		return
	}
	parsed, _ := url.Parse(server.URL)
	request, _ := http.NewRequestWithContext(ctx, "GET", parsed.Scheme+"://"+parsed.Host+"/web/v1/session", nil)
	request.AddCookie(cookie)
	response, err = client.Do(request)
	add("demo_session_authenticates_gateway", err == nil && response.StatusCode == 200)
	if err == nil {
		response.Body.Close()
	}
	for _, origin := range []string{"", "https://foreign.example", "http://localhost:1", w.Gateway.URL} {
		mutation, _ := http.NewRequestWithContext(ctx, "POST", parsed.Scheme+"://"+parsed.Host+"/web/v1/logout", nil)
		mutation.AddCookie(cookie)
		if origin != "" {
			mutation.Header.Set("Origin", origin)
		}
		result, problem := client.Do(mutation)
		add(fmt.Sprintf("demo_proxy_rejects_invalid_origin_%d", len(origin)), problem == nil && result.StatusCode == 403)
		if problem == nil {
			result.Body.Close()
		}
	}
	response, err = client.Get(server.URL)
	add("demo_link_single_use", err == nil && response.StatusCode == 410)
	if err == nil {
		response.Body.Close()
	}
	request, _ = http.NewRequestWithContext(ctx, "GET", w.Gateway.URL+parsed.Path, nil)
	response, err = client.Do(request)
	noSession := err == nil
	if err == nil {
		noSession = noSession && len(response.Cookies()) == 0
		response.Body.Close()
	}
	add("product_gateway_cannot_create_demo_session", noSession)
	invalid, err := demo.Serve(w, "0.0.0.0:0")
	if invalid != nil {
		invalid.Close()
	}
	add("demo_rejects_non_loopback_listener", err != nil)
}
