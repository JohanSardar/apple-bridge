package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"
	"time"

	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func uploadReplicaBrowserClient(ctx context.Context, w *world.World, add func(string, bool)) (*webClient, error) {
	origin := strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1)
	control, key := webBrowserControl(ctx, w, origin)
	defer control.Close()
	bounded, stop := context.WithTimeout(ctx, 30*time.Second)
	defer stop()
	request, _ := http.NewRequestWithContext(bounded, "POST", control.URL+"/replica-session", nil)
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		return nil, err
	}
	response.Body.Close()
	add("replica_session_hook_requires_control_secret", response.StatusCode == 403 && len(w.Replicas) == 0)
	request, _ = http.NewRequestWithContext(bounded, "POST", control.URL+"/replica-session", nil)
	request.Header.Set("X-Bridge-Lab", key)
	response, err = http.DefaultClient.Do(request)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	var output struct {
		URL    string `json:"url"`
		Cookie struct {
			Name     string `json:"name"`
			Value    string `json:"value"`
			URL      string `json:"url"`
			HTTPOnly bool   `json:"httpOnly"`
			Secure   bool   `json:"secure"`
			SameSite string `json:"sameSite"`
		} `json:"cookie"`
	}
	decoder := json.NewDecoder(io.LimitReader(response.Body, 64<<10))
	decoder.DisallowUnknownFields()
	err = decoder.Decode(&output)
	valid := err == nil && response.StatusCode == 200 && len(w.Replicas) == 1 && strings.HasPrefix(output.URL, "http://localhost:") && output.Cookie.URL == output.URL && output.Cookie.Name == bridgeweb.LocalSessionCookie && output.Cookie.Value != "" && output.Cookie.HTTPOnly && !output.Cookie.Secure && output.Cookie.SameSite == "Strict"
	add("replica_session_hook_returns_only_url_and_cookie", valid)
	if !valid {
		return nil, fmt.Errorf("replica session fixture invalid")
	}
	client := &webClient{ctx: ctx, origin: output.URL, cookie: &http.Cookie{Name: output.Cookie.Name, Value: output.Cookie.Value}}
	response, _, err = client.request("GET", "session", nil)
	add("replica_browser_session_authorized", err == nil && response.StatusCode == 200)
	if err != nil {
		return nil, err
	}
	return client, nil
}
