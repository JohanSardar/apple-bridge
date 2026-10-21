package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/coder/websocket"
	"github.com/coder/websocket/wsjson"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing/fstest"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runWeb(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if level != "sim" {
		add("Playwright runner pending W1-UI", false)
		return e
	}
	w, err := world.New(ctx, level, 0, 1, world.BlobOptions{Scope: "admin"})
	if err != nil {
		add("world_start: "+err.Error(), false)
		return e
	}
	defer w.Close()
	request := func(method, path, origin string) (*http.Response, string) {
		req, _ := http.NewRequestWithContext(ctx, method, w.Gateway.URL+path, strings.NewReader("{}"))
		req.Host = w.Gateway.Host
		if origin != "" {
			req.Header.Set("Origin", origin)
		}
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			add("http_request", false)
			return nil, ""
		}
		defer resp.Body.Close()
		body, _ := io.ReadAll(resp.Body)
		return resp, string(body)
	}
	if s.ID == "WEB-AUTH-SOFTWARE" || s.ID == "WEB-LIVE-RECORD" || s.ID == "WEB-BLOB-ACCESS" || s.ID == "WEB-COMMAND-INTERACTIVE" {
		runWebSession(ctx, w, s.ID, &e, add)
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
		return e
	}
	switch s.ID {
	case "WEB-SERVE-EMBED":
		for _, built := range []bool{false, true} {
			assets := fstest.MapFS{}
			if built {
				assets["dist/index.html"] = &fstest.MapFile{Data: []byte("<html>Bridge Test</html>")}
				assets["dist/assets/test-hash.js"] = &fstest.MapFile{Data: []byte("export const ready=true")}
				assets["dist/sw.js"] = &fstest.MapFile{Data: []byte("self.addEventListener('fetch',()=>{})")}
				assets["dist/manifest.webmanifest"] = &fstest.MapFile{Data: []byte("{}")}
			}
			fixture := httptest.NewServer(bridgeweb.Handler(bridgeweb.Deps{Assets: assets}))
			paths := []string{"/", "/messages/test"}
			if built {
				paths = append(paths, "/assets/test-hash.js", "/sw.js", "/manifest.webmanifest")
			}
			for _, path := range paths {
				resp, err := http.Get(fixture.URL + path)
				if err != nil {
					add("static_fixture", false)
					continue
				}
				body, _ := io.ReadAll(resp.Body)
				resp.Body.Close()
				cache := "no-store"
				if strings.HasPrefix(path, "/assets/") {
					cache = "public, max-age=31536000, immutable"
				} else if path == "/sw.js" || path == "/manifest.webmanifest" {
					cache = "no-cache"
				}
				add("static_cache_"+path, resp.StatusCode == 200 && resp.Header.Get("Cache-Control") == cache)
				if !built {
					add("empty_dist_not_built", strings.Contains(string(body), "make web"))
				}
			}
			fixture.Close()
		}
		resp, body := request("GET", "/", "")
		add("app_or_not_built", resp != nil && resp.StatusCode == 200 && (strings.Contains(body, "make web") || strings.Contains(body, "<html")))
		add("index_no_store", resp != nil && resp.Header.Get("Cache-Control") == "no-store")
		resp, _ = request("GET", "/messages/test", "")
		add("app_route_fallback", resp != nil && resp.StatusCode == 200)
	case "WEB-AUTH-ORIGIN":
		for _, origin := range []string{"", "https://foreign.example"} {
			for _, path := range []string{"/web/v1/login/options", "/web/v1/enroll/options", "/web/v1/logout"} {
				resp, _ := request("POST", path, origin)
				add("reject_origin_"+path, resp != nil && resp.StatusCode == 403)
			}
			resp, _ := request("GET", "/web/v1/live", origin)
			add("reject_ws_origin", resp != nil && resp.StatusCode == 403)
		}
		resp, _ := request("GET", "/", "")
		add("security_headers", resp != nil && resp.Header.Get("Content-Security-Policy") != "" && resp.Header.Get("Referrer-Policy") == "no-referrer" && resp.Header.Get("X-Content-Type-Options") == "nosniff")
	case "WEB-AUTH-RATELIMIT":
		statuses := []int{}
		for i := 0; i < 11; i++ {
			resp, _ := request("POST", "/web/v1/login/options", "http://"+w.Gateway.Host)
			if resp != nil {
				statuses = append(statuses, resp.StatusCode)
			}
		}
		add("signin_minute_limit", len(statuses) == 11 && statuses[10] == 429)
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

type webClient struct {
	ctx    context.Context
	origin string
	cookie *http.Cookie
}

func (c *webClient) request(method, path string, input any) (*http.Response, []byte, error) {
	raw := []byte{}
	if input != nil {
		raw, _ = json.Marshal(input)
	}
	req, err := http.NewRequestWithContext(c.ctx, method, c.origin+"/web/v1/"+path, bytes.NewReader(raw))
	if err != nil {
		return nil, nil, err
	}
	req.Header.Set("Origin", c.origin)
	req.Header.Set("Content-Type", "application/json")
	if c.cookie != nil {
		req.AddCookie(c.cookie)
	}
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, nil, err
	}
	defer resp.Body.Close()
	body, err := io.ReadAll(resp.Body)
	return resp, body, err
}
func (c *webClient) signIn(auth *webAuthenticator, flow, token, kind string) (int, []byte, error) {
	resp, raw, err := c.request("POST", flow+"/options", map[string]string{"token": token, "kind": kind})
	if err != nil {
		return 0, nil, err
	}
	if resp.StatusCode != 200 {
		return resp.StatusCode, raw, fmt.Errorf("options status %d", resp.StatusCode)
	}
	var options struct {
		ID        string `json:"ceremony_id"`
		PublicKey struct {
			Challenge string `json:"challenge"`
		} `json:"publicKey"`
	}
	if err = json.Unmarshal(raw, &options); err != nil {
		return 0, nil, err
	}
	origin, _ := url.Parse(c.origin)
	credential, err := auth.credential(flow != "login", options.PublicKey.Challenge, c.origin, origin.Hostname())
	if err != nil {
		return 0, nil, err
	}
	resp, body, err := c.request("POST", flow+"/finish", map[string]any{"ceremony_id": options.ID, "credential": credential})
	if err != nil {
		return 0, nil, err
	}
	for _, cookie := range resp.Cookies() {
		if cookie.Name == bridgeweb.SessionCookie {
			c.cookie = cookie
		}
	}
	return resp.StatusCode, body, nil
}
func (c *webClient) socket() (*websocket.Conn, error) {
	header := http.Header{"Origin": []string{c.origin}}
	if c.cookie != nil {
		header.Set("Cookie", c.cookie.String())
	}
	conn, _, err := websocket.Dial(c.ctx, strings.Replace(c.origin, "http://", "ws://", 1)+"/web/v1/live", &websocket.DialOptions{HTTPHeader: header, Subprotocols: []string{"web.1"}})
	if err != nil {
		return nil, err
	}
	if err = wsjson.Write(c.ctx, conn, map[string]any{"t": "hello", "v": 1}); err != nil {
		conn.CloseNow()
		return nil, err
	}
	var frame map[string]any
	if err = wsjson.Read(c.ctx, conn, &frame); err != nil || frame["t"] != "welcome" {
		conn.CloseNow()
		return nil, fmt.Errorf("welcome failed: %v", err)
	}
	return conn, nil
}
func runWebSession(ctx context.Context, w *world.World, id string, e *Evidence, add func(string, bool)) {
	_, port, _ := strings.Cut(w.Gateway.Host, ":")
	c := &webClient{ctx: ctx, origin: "http://localhost:" + port}
	auth, err := newWebAuthenticator()
	if err != nil {
		add("authenticator", false)
		return
	}
	out, err := exec.CommandContext(ctx, "bin/bridge", "web", "enroll", "--home", w.Gateway.Home, "--host", c.origin).Output()
	if err != nil {
		add("enroll_cli", false)
		return
	}
	enrollURL, err := url.Parse(strings.TrimSpace(string(out)))
	if err != nil {
		add("enroll_url", false)
		return
	}
	status, body, err := c.signIn(auth, "enroll", enrollURL.Fragment, "trusted")
	add("software_passkey_enrollment", err == nil && status == 200)
	if err != nil || status != 200 {
		e.Assertions = append(e.Assertions, Assertion{Name: "enrollment_status", Detail: fmt.Sprintf("status=%d error=%v body=%s", status, err, string(body)), Passed: false})
		return
	}
	add("host_cookie_flags", c.cookie != nil && c.cookie.Name == "__Host-ab_session" && c.cookie.Path == "/" && c.cookie.Secure && c.cookie.HttpOnly && c.cookie.SameSite == http.SameSiteStrictMode && c.cookie.MaxAge > 0 && c.cookie.Domain == "")
	if id == "WEB-BLOB-ACCESS" {
		runWebBlobs(ctx, w, c, e, add)
		return
	}
	conn, err := c.socket()
	add("web1_hello_welcome", err == nil && conn != nil && conn.Subprotocol() == "web.1")
	resp, _, protocolErr := c.request("GET", "live", nil)
	add("upgrade_requires_web_dot_one", protocolErr == nil && resp.StatusCode == 400)
	if err != nil {
		return
	}
	defer conn.CloseNow()
	read := func(conn *websocket.Conn) (map[string]any, error) {
		bounded, cancel := context.WithTimeout(ctx, 2*time.Second)
		defer cancel()
		var frame map[string]any
		err := wsjson.Read(bounded, conn, &frame)
		return frame, err
	}
	if id == "WEB-COMMAND-INTERACTIVE" {
		var lists contracts.ReadResult
		err := w.Await(5*time.Second, func() bool {
			_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.list"}, &lists)
			return err == nil && len(lists.Records) > 0
		})
		if err != nil || len(lists.Records) == 0 {
			add("reminders_list", false)
			return
		}
		commandID := contracts.NewCommandID()
		wsjson.Write(ctx, conn, map[string]any{"t": "cmd", "id": commandID, "op": "reminders.create", "args": map[string]any{"list_id": lists.Records[0].ID, "title": "[ab-test] web command"}})
		frame, err := read(conn)
		add("command_ack", err == nil && frame["t"] == "ack")
		frame, err = read(conn)
		add("command_status", err == nil && frame["t"] == "cmd_status")
		raw, _ := json.Marshal(frame["command"])
		var result contracts.CommandRecord
		json.Unmarshal(raw, &result)
		add("interactive_in_command_ledger", result.Command.ID == commandID && result.Command.Origin == "interactive" && strings.HasPrefix(result.Principal, "device:"))
		for result.Disposition == "" && err == nil {
			frame, err = read(conn)
			raw, _ = json.Marshal(frame["command"])
			json.Unmarshal(raw, &result)
		}
		add("command_reaches_final_status", err == nil && result.Disposition != "")
		wsjson.Write(ctx, conn, map[string]any{"t": "cmd", "id": commandID, "op": "reminders.create", "args": map[string]any{"list_id": lists.Records[0].ID, "title": "[ab-test] web command"}})
		_, err = read(conn)
		frame, err = read(conn)
		raw, _ = json.Marshal(frame["command"])
		var duplicate contracts.CommandRecord
		json.Unmarshal(raw, &duplicate)
		add("command_resend_idempotent", err == nil && duplicate.Command.ID == commandID && duplicate.CreatedAt.Equal(result.CreatedAt))
		response, err := w.Control.Get("http://sim/command-origins")
		origins := map[string]map[string]string{}
		if err == nil {
			json.NewDecoder(response.Body).Decode(&origins)
			response.Body.Close()
		}
		add("interactive_crosses_ipc_as_data", err == nil && origins[commandID]["Prepare"] == "interactive" && origins[commandID]["Dispatch"] == "interactive" && origins[commandID]["action"] == "interactive")
		effects, err := w.Effects()
		add("one_native_effect", err == nil && effects[commandID] == 1)
		return
	}
	if id == "WEB-AUTH-SOFTWARE" {
		resp, _, err := c.request("POST", "enroll/options", map[string]any{"token": enrollURL.Fragment})
		add("enroll_single_use", err == nil && resp.StatusCode == 401)
		public := &webClient{ctx: ctx, origin: c.origin}
		status, body, err := public.signIn(auth, "login", "", "public")
		add("discoverable_public_signin", err == nil && status == 200 && bytes.Contains(body, []byte(`"cache":"memory"`)))
		if err != nil || status != 200 {
			return
		}
		add("public_session_cookie", public.cookie != nil && public.cookie.MaxAge == 0 && public.cookie.Expires.IsZero() && public.cookie.Secure && public.cookie.HttpOnly)
		frame, err := read(conn)
		add("signin_to_existing_session", err == nil && frame["t"] == "signin")
		other, err := public.socket()
		add("public_socket", err == nil)
		if err != nil {
			return
		}
		defer other.CloseNow()
		var session struct {
			ID string `json:"device_id"`
		}
		json.Unmarshal(body, &session)
		at := time.Now()
		resp, _, err = c.request("DELETE", "devices/"+session.ID, nil)
		frame, readErr := read(other)
		elapsed := time.Since(at)
		add("revoke_notifies_within_second", err == nil && resp.StatusCode == 200 && readErr == nil && frame["t"] == "session_revoked" && elapsed < time.Second)
		e.Latency["session_revoke"] = stats([]float64{float64(elapsed) / float64(time.Millisecond)}, 0)
		_, _, closeErr := other.Read(ctx)
		add("revoke_closes_socket", closeErr != nil)
		resp, _, err = public.request("GET", "session", nil)
		add("revoked_cookie_rejected", err == nil && resp.StatusCode == 401)
		_, err = exec.CommandContext(ctx, "bin/bridge", "web", "lock", "--home", w.Gateway.Home).Output()
		add("lock_cli", err == nil)
		frame, err = read(conn)
		add("lock_revokes_live_session", err == nil && frame["t"] == "session_revoked")
		return
	}
	if err = w.ControlAction("/messages", map[string]any{"action": "seed", "count": 1}); err != nil {
		add("message_seed", false)
		return
	}
	var records contracts.ReadResult
	err = w.Await(5*time.Second, func() bool {
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.message"}, &records)
		return err == nil && len(records.Records) == 1
	})
	if err != nil {
		add("message_seed_visible", false)
		return
	}
	record := records.Records[0]
	os.WriteFile(filepath.Join(w.Gateway.Home, "lab-web-metrics"), nil, 0600)
	wsjson.Write(ctx, conn, map[string]any{"t": "sub", "id": "message", "view": "record", "args": map[string]any{"collection": record.Collection, "id": record.ID}})
	frame, err := read(conn)
	add("record_snap", err == nil && frame["t"] == "snap")
	if err != nil {
		return
	}
	var lastCursor any
	latencies := []float64{}
	for i := 0; i < 32; i++ {
		action := "receipts"
		if i == 0 {
			action = "incoming"
		}
		err = w.ControlAction("/messages", map[string]any{"action": action, "key": "message-000000"})
		if err != nil {
			add("inbound_sim_message", false)
			return
		}
		delivered := false
		for attempts := 0; attempts < 8; attempts++ {
			frame, err = read(conn)
			if err != nil {
				break
			}
			if frame["t"] == "delta" {
				changes, _ := frame["changes"].([]any)
				if len(changes) > 0 {
					lastCursor = frame["cursor"]
					delivered = true
					break
				}
			}
		}
		if !delivered {
			add("journal_record_delta", false)
			return
		}
	}
	data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "web-metrics.jsonl"))
	commits := map[uint64]int64{}
	writes := map[uint64]int64{}
	for _, line := range bytes.Split(data, []byte{'\n'}) {
		var metric struct {
			Stage   string `json:"stage"`
			Through uint64 `json:"through"`
			At      int64  `json:"at"`
		}
		if json.Unmarshal(line, &metric) != nil {
			continue
		}
		if metric.Stage == "commit" {
			commits[metric.Through] = metric.At
		} else {
			writes[metric.Through] = metric.At
		}
	}
	for seq, at := range commits {
		if written, ok := writes[seq]; ok && written >= at {
			latencies = append(latencies, float64(written-at)/float64(time.Millisecond))
		}
	}
	e.Latency["journal_commit_to_frame_written"] = stats(latencies, 0)
	add("inbound_and_updates_from_journal", err == nil && len(latencies) == 32)
	add("commit_to_frame_p95_20ms", len(latencies) == 32 && stats(latencies, 0).P95 <= 20)
	conn.CloseNow()
	err = w.ControlAction("/messages", map[string]any{"action": "receipts", "key": "message-000000"})
	if err != nil {
		add("offline_change", false)
		return
	}
	resumed, err := c.socket()
	add("reconnect", err == nil)
	if err != nil {
		return
	}
	defer resumed.CloseNow()
	wsjson.Write(ctx, resumed, map[string]any{"t": "sub", "id": "resumed", "view": "record", "args": map[string]any{"collection": record.Collection, "id": record.ID}, "cursor": lastCursor})
	frame, err = read(resumed)
	add("resume_replays_without_snapshot", err == nil && frame["t"] == "delta")
}

func runWebBlobs(ctx context.Context, w *world.World, c *webClient, e *Evidence, add func(string, bool)) {
	if err := w.ControlAction("/blob-fixture", map[string]any{"size": 1024}); err != nil {
		add("blob_fixture", false)
		return
	}
	var records contracts.ReadResult
	err := w.Await(5*time.Second, func() bool {
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.attachment"}, &records)
		return err == nil && len(records.Records) == 1
	})
	if err != nil {
		add("attachment_visible", false)
		return
	}
	record := records.Records[0]
	refs := contracts.BlobReferences(record.Data)
	if len(refs) == 0 {
		add("attachment_blob_reference", false)
		return
	}
	digest := refs[0].Digest
	path := "blob/" + digest + "?record=" + record.ID
	var resp *http.Response
	var body []byte
	err = w.Await(5*time.Second, func() bool {
		resp, body, err = c.request("GET", path, nil)
		return err == nil && resp.StatusCode == 200
	})
	add("authorized_blob", err == nil && len(body) == 1024)
	if err != nil {
		return
	}
	add("blob_immutable_private_etag", resp.Header.Get("Cache-Control") == "private, max-age=31536000, immutable" && resp.Header.Get("ETag") == `"`+digest+`"`)
	req, _ := http.NewRequestWithContext(ctx, "GET", c.origin+"/web/v1/"+path, nil)
	req.AddCookie(c.cookie)
	req.Header.Set("Range", "bytes=10-19")
	resp, err = http.DefaultClient.Do(req)
	if err == nil {
		body, _ = io.ReadAll(resp.Body)
		resp.Body.Close()
	}
	add("blob_range", err == nil && resp.StatusCode == 206 && len(body) == 10)
	resp, _, err = c.request("GET", "blob/"+digest+"?record=unrelated", nil)
	add("digest_alone_denied", err == nil && resp.StatusCode == 401)
	resp, _, err = c.request("GET", path+"&variant=display", nil)
	add("unsafe_original_variant_pending", err == nil && resp.StatusCode == 409)
	resp, body, err = c.request("POST", "upload", map[string]string{"synthetic": "Bridge Test"})
	add("streamed_upload_staged", err == nil && resp.StatusCode == 200 && bytes.Contains(body, []byte("upload_id")))
}
