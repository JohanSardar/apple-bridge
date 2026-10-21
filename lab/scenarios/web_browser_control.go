package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webBrowserControl(ctx context.Context, w *world.World, origin string) (*httptest.Server, string) {
	secret := contracts.NewID("browser")
	var mu sync.Mutex
	var browserReplica *world.Endpoint
	enroll := func(expired bool) (string, error) {
		output, err := exec.CommandContext(ctx, "bin/bridge", "web", "enroll", "--home", w.Gateway.Home, "--host", origin).Output()
		if err != nil {
			return "", fmt.Errorf("enroll CLI: %w", err)
		}
		link := strings.TrimSpace(string(output))
		if expired {
			parsed, err := url.Parse(link)
			if err != nil {
				return "", err
			}
			path := filepath.Join(w.Gateway.Home, "credentials.json")
			raw, err := os.ReadFile(path)
			if err != nil {
				return "", err
			}
			var fixture map[string]any
			if err := json.Unmarshal(raw, &fixture); err != nil {
				return "", err
			}
			tokens, ok := fixture["web_enroll"].(map[string]any)
			if !ok {
				return "", fmt.Errorf("enroll fixture missing")
			}
			hash := sha256.Sum256([]byte(parsed.Fragment))
			tokens[hex.EncodeToString(hash[:])] = time.Now().Add(-time.Minute).Format(time.RFC3339Nano)
			encoded, err := json.Marshal(fixture)
			if err != nil {
				return "", err
			}
			if err := os.WriteFile(path+".browser-expiry", encoded, 0600); err != nil {
				return "", err
			}
			if err := os.Rename(path+".browser-expiry", path); err != nil {
				return "", err
			}
		}
		return link, nil
	}
	handler := http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.Method != "POST" || request.Header.Get("X-Bridge-Lab") != secret {
			http.Error(response, "forbidden", 403)
			return
		}
		mu.Lock()
		defer mu.Unlock()
		response.Header().Set("Content-Type", "application/json")
		var value any
		var err error
		switch request.URL.Path {
		case "/replica-upload-checkpoint/arm", "/replica-upload-checkpoint/wait", "/replica-upload-checkpoint/release", "/command-effect":
			if !webUploadCheckpointControl(response, request, w) {
				http.NotFound(response, request)
			}
			return
		case "/enroll":
			var link string
			link, err = enroll(request.URL.Query().Get("expired") == "1")
			value = map[string]any{"url": link}
		case "/local-credential":
			var link string
			link, err = enroll(false)
			if err != nil {
				break
			}
			var parsed *url.URL
			parsed, err = url.Parse(link)
			if err != nil {
				break
			}
			client := webClient{ctx: ctx, origin: origin}
			var response *http.Response
			var data []byte
			response, data, err = client.request("POST", "enroll/options", map[string]string{"token": parsed.Fragment, "kind": "trusted"})
			if err != nil {
				break
			}
			if response.StatusCode != 200 {
				err = fmt.Errorf("enrollment options status %d", response.StatusCode)
				break
			}
			var options struct {
				ID        string `json:"ceremony_id"`
				PublicKey struct {
					Challenge string `json:"challenge"`
				} `json:"publicKey"`
			}
			err = json.Unmarshal(data, &options)
			if err != nil {
				break
			}
			var authenticator *webAuthenticator
			authenticator, err = newWebAuthenticator()
			if err != nil {
				break
			}
			var credential map[string]any
			credential, err = authenticator.credential(true, options.PublicKey.Challenge, origin, parsed.Hostname())
			value = map[string]any{"ceremony_id": options.ID, "credential": credential}

		case "/cookie-boundaries":
			var input struct {
				Token string `json:"token"`
			}
			err = json.NewDecoder(request.Body).Decode(&input)
			if err == nil {
				value, err = webCookieBoundaries(ctx, w, input.Token)
			}
		case "/replica-session":
			value, err = webReplicaSession(ctx, w, &browserReplica)
			if err != nil {
				err = fmt.Errorf("replica session unavailable")
			}
		case "/session":
			var link string
			link, err = enroll(false)
			if err != nil {
				break
			}
			var parsed *url.URL
			parsed, err = url.Parse(link)
			if err != nil {
				break
			}
			var authenticator *webAuthenticator
			authenticator, err = newWebAuthenticator()
			if err != nil {
				break
			}
			client := webClient{ctx: ctx, origin: origin}
			var status int
			var body []byte
			status, body, err = client.signIn(authenticator, "enroll", parsed.Fragment, "trusted")
			if err == nil && status != 200 {
				err = fmt.Errorf("software enrollment status %d", status)
			}
			if err != nil {
				break
			}
			if request.URL.Query().Get("kind") == "public" {
				status, body, err = client.signIn(authenticator, "login", "", "public")
				if err == nil && status != 200 {
					err = fmt.Errorf("software sign-in status %d", status)
				}
			}
			if err != nil {
				break
			}
			var session map[string]any
			err = json.Unmarshal(body, &session)
			if err == nil && client.cookie == nil {
				err = fmt.Errorf("session cookie missing")
			}
			if err != nil {
				break
			}
			value = map[string]any{"session": session, "cookie": map[string]any{"name": client.cookie.Name, "value": client.cookie.Value, "url": origin, "httpOnly": true, "secure": client.cookie.Secure, "sameSite": "Strict"}}
		case "/contact-avatar":
			value, err = webAvatarFixture(w)
		case "/demo-fixtures":
			value, err = demo.Fixtures(ctx, w)
		case "/handles-fixture":
			value, err = webHandlesFixture(w)
		case "/list-fixture":
			started := time.Now()
			avatar, setupErr := webListFixture(w, request.URL.Query().Get("update") == "1")
			err = setupErr
			value = map[string]any{"ok": err == nil, "fixture_setup_ms": time.Since(started).Milliseconds(), "fixture_flush_ms": avatar.FixtureFlushMS, "fixture_flush_budget_ms": avatar.FixtureFlushBudgetMS}
		case "/list-late-incoming":
			err = webListLateIncoming(w)
			value = map[string]any{"ok": err == nil}
		case "/message-fault":
			name := request.URL.Query().Get("name")
			switch name {
			case "", "ghost-send", "delivery-failed", "ui-interference", "part-progress":
				err = w.ControlAction("/fault", map[string]any{"name": name})
			default:
				http.NotFound(response, request)
				return
			}
			value = map[string]any{"ok": err == nil}
		case "/incoming":
			started := time.Now().UnixMilli()
			key := contracts.NewID("web-incoming")
			err = w.ControlAction("/messages", map[string]any{"action": "incoming", "key": key})
			value = map[string]any{"started_at_ms": started}
		case "/capability-unproven", "/capability-proven", "/capability-proven-empty-reason":
			err = w.ControlAction("/messages", map[string]any{"action": strings.TrimPrefix(request.URL.Path, "/")})
			value = map[string]any{"ok": err == nil}
		case "/lock":
			_, err = exec.CommandContext(ctx, "bin/bridge", "web", "lock", "--home", w.Gateway.Home).Output()
			value = map[string]any{"ok": err == nil}
		default:
			http.NotFound(response, request)
			return
		}
		if err != nil {
			http.Error(response, err.Error(), 500)
			return
		}
		json.NewEncoder(response).Encode(value)
	})
	return httptest.NewServer(handler), secret
}
