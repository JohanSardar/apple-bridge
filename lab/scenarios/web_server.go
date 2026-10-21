package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/trust"
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
	"sync/atomic"
	"testing/fstest"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runWebServer(ctx context.Context, s Scenario, level string) Evidence {
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
	if s.ID == "WEB-AUTH-SOFTWARE" || s.ID == "WEB-LIVE-RECORD" || s.ID == "WEB-BLOB-ACCESS" || s.ID == "WEB-COMMAND-INTERACTIVE" || s.ID == "WEB-LIVE-READMODELS" {
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
		secure := httptest.NewTLSServer(bridgeweb.Handler(bridgeweb.Deps{Assets: fstest.MapFS{}}))
		tlsResponse, tlsErr := secure.Client().Get(secure.URL + "/")
		add("hsts_on_https", tlsErr == nil && tlsResponse.Header.Get("Strict-Transport-Security") != "")
		if tlsErr == nil {
			tlsResponse.Body.Close()
		}
		secure.Close()
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
		rateClient := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1)}
		for i := 0; i < 11; i++ {
			resp, _, _ := rateClient.request("POST", "login/options", map[string]any{})
			if resp != nil {
				statuses = append(statuses, resp.StatusCode)
			}
		}
		available := len(statuses) == 11
		if available {
			for _, status := range statuses[:10] {
				available = available && status == 200
			}
		}
		add("signin_options_available", available)
		add("signin_minute_limit", len(statuses) == 11 && statuses[10] == 429)
		runWebRateWindows(ctx, w, add)
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
	var trustedSession struct {
		Expires time.Time `json:"expires_at"`
	}
	json.Unmarshal(body, &trustedSession)
	add("trusted_thirty_days", time.Until(trustedSession.Expires) > 29*24*time.Hour && time.Until(trustedSession.Expires) <= 30*24*time.Hour)
	decodedToken, decodeErr := base64.RawURLEncoding.DecodeString(c.cookie.Value)
	add("session_256_bits", decodeErr == nil && len(decodedToken) == 32)
	stored, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "credentials.json"))
	add("tokens_stored_hashed", !bytes.Contains(stored, []byte(c.cookie.Value)) && !bytes.Contains(stored, []byte(enrollURL.Fragment)))
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
	if id == "WEB-LIVE-READMODELS" {
		runWebReadModels(ctx, w, c, conn, e, add)
		return
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
		wsjson.Write(ctx, conn, map[string]any{"t": "sub", "id": "command", "view": "record", "args": map[string]any{"collection": "system.command", "id": commandID}})
		frame, err = read(conn)
		data, _ := frame["data"].(map[string]any)
		rows, _ := data["records"].([]any)
		reconciled := false
		if len(rows) == 1 {
			record, _ := rows[0].(map[string]any)
			status, _ := record["data"].(map[string]any)
			reconciled = status["state"] == string(result.State) && status["disposition"] == string(result.Disposition)
		}
		add("command_record_reconnect_reconciles_ledger", err == nil && frame["t"] == "snap" && reconciled)
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
		var publicSession struct {
			Expires time.Time `json:"expires_at"`
		}
		json.Unmarshal(body, &publicSession)
		add("public_thirty_minute_idle", time.Until(publicSession.Expires) > 29*time.Minute && time.Until(publicSession.Expires) <= 30*time.Minute)
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
		expiredURL, expireErr := exec.CommandContext(ctx, "bin/bridge", "web", "enroll", "--home", w.Gateway.Home, "--host", c.origin).Output()
		parsedExpired, _ := url.Parse(strings.TrimSpace(string(expiredURL)))
		if expireErr == nil && parsedExpired != nil {
			path := filepath.Join(w.Gateway.Home, "credentials.json")
			stored, readErr := os.ReadFile(path)
			var fixture map[string]any
			json.Unmarshal(stored, &fixture)
			hash := sha256.Sum256([]byte(parsedExpired.Fragment))
			if tokens, ok := fixture["web_enroll"].(map[string]any); ok {
				tokens[hex.EncodeToString(hash[:])] = time.Now().Add(-time.Minute).Format(time.RFC3339Nano)
			}
			modified, _ := json.Marshal(fixture)
			writeErr := os.WriteFile(path+".web-expiry", modified, 0600)
			if writeErr == nil {
				writeErr = os.Rename(path+".web-expiry", path)
			}
			resp, _, err = c.request("POST", "enroll/options", map[string]any{"token": parsedExpired.Fragment})
			add("expired_enroll_token_denied", readErr == nil && writeErr == nil && err == nil && resp.StatusCode == 401)
			auth.UserVerified = false
			invalid := &webClient{ctx: ctx, origin: c.origin}
			status, _, err = invalid.signIn(auth, "login", "", "trusted")
			add("user_verification_required", err == nil && status == 401 && invalid.cookie == nil)
			auth.UserVerified = true
		} else {
			add("expired_enroll_fixture", false)
		}
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

	for _, size := range []int64{100 * 1024 * 1024, 100*1024*1024 + 1} {
		request, _ := http.NewRequestWithContext(ctx, "POST", c.origin+"/web/v1/upload", io.LimitReader(webZeroReader{}, size))
		request.Header.Set("Origin", c.origin)
		request.AddCookie(c.cookie)
		response, err := http.DefaultClient.Do(request)
		expected := 200
		if size > 100*1024*1024 {
			expected = 413
		}
		add(fmt.Sprintf("stream_upload_%d_bytes", size), err == nil && response.StatusCode == expected)
		if err == nil {
			io.Copy(io.Discard, response.Body)
			response.Body.Close()
		}
	}
	for _, fixture := range []struct {
		mime             string
		variant, missing bool
		status           int
		name             string
	}{
		{"image/heic", true, false, 200, "derived_display_bytes"},
		{"image/heic", true, true, 409, "missing_derivative_pending"},
		{"image/png", true, true, 200, "missing_derivative_safe_original"},
	} {
		previousVersion := record.Version
		err = w.ControlAction("/blob-fixture", map[string]any{"size": 1024, "mime_type": fixture.mime, "variant": fixture.variant, "missing_variant": fixture.missing})
		if err != nil {
			add(fixture.name+"_fixture", false)
			return
		}
		err = w.Await(5*time.Second, func() bool {
			_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.attachment"}, &records)
			if err != nil || len(records.Records) != 1 {
				return false
			}
			record = records.Records[0]
			return record.Version > previousVersion
		})
		var metadata struct {
			Blob struct {
				Digest string `json:"digest"`
			} `json:"blob"`
		}
		json.Unmarshal(record.Data, &metadata)
		path = "blob/" + metadata.Blob.Digest + "?record=" + record.ID + "&variant=display"
		err = w.Await(5*time.Second, func() bool {
			resp, body, err = c.request("GET", path, nil)
			return err == nil && resp.StatusCode == fixture.status
		})
		valid := err == nil
		if fixture.status == 200 {
			valid = valid && resp.Header.Get("Content-Type") == "image/png" && len(body) > 8 && bytes.Equal(body[:8], []byte{137, 80, 78, 71, 13, 10, 26, 10})
		}
		add(fixture.name, valid)
	}

}

func runWebReadModels(ctx context.Context, w *world.World, c *webClient, conn *websocket.Conn, e *Evidence, add func(string, bool)) {
	if err := w.ControlAction("/messages", map[string]any{"action": "seed", "count": 3}); err != nil {
		add("readmodel_seed", false)
		return
	}
	var chats contracts.ReadResult
	err := w.Await(5*time.Second, func() bool {
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.chat"}, &chats)
		return err == nil && len(chats.Records) == 1
	})
	if err != nil {
		add("chat_visible", false)
		return
	}
	chatID := chats.Records[0].ID
	var initialMessages contracts.ReadResult
	_, err = world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.message"}, &initialMessages)
	if err != nil || len(initialMessages.Records) != 3 {
		add("thread_window_fixture", false)
		return
	}
	read := func() (map[string]any, error) {
		bounded, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		var frame map[string]any
		err := wsjson.Read(bounded, conn, &frame)
		return frame, err
	}
	for _, view := range []string{"chats", "thread", "collection"} {
		args := map[string]any{}
		if view != "chats" {
			args["chat_id"] = chatID
		}
		if view == "thread" {
			args["anchor"] = initialMessages.Records[1].ID
			args["limit"] = 3
		}
		if view == "collection" {
			args["kind"] = "photos"
		}
		wsjson.Write(ctx, conn, map[string]any{"t": "sub", "id": view, "view": view, "args": args})
		found := false
		for tries := 0; tries < 12; tries++ {
			frame, err := read()
			if err != nil {
				break
			}
			if frame["t"] == "error" {
				add("readmodel_"+view+"_available", false)
				return
			}
			if frame["t"] == "snap" && frame["sub"] == view {
				found = true
				data, _ := frame["data"].(map[string]any)
				field := "items"
				count := 0
				if view == "chats" {
					field = "chats"
					count = 1
				} else if view == "thread" {
					count = 3
				}
				rows, _ := data[field].([]any)
				add("readmodel_"+view+"_snapshot_rows", len(rows) == count)
				break
			}
		}
		add("readmodel_"+view+"_snapshot", found)
		if !found {
			return
		}
	}

	wsjson.Write(ctx, conn, map[string]any{"t": "sub", "id": "edge", "view": "thread", "args": map[string]any{"chat_id": chatID, "anchor": initialMessages.Records[0].ID, "limit": 1}})
	foundEdge := false
	for tries := 0; tries < 12; tries++ {
		frame, err := read()
		if err != nil {
			break
		}
		if frame["t"] == "snap" && frame["sub"] == "edge" {
			data, _ := frame["data"].(map[string]any)
			newer, _ := data["newer"].(float64)
			older, _ := data["older"].(float64)
			foundEdge = newer == 2 && older == 0
			break
		}
	}
	add("initial_thread_outside_counts", foundEdge)
	wsjson.Write(ctx, conn, map[string]any{"t": "unsub", "id": "edge"})
	os.WriteFile(filepath.Join(w.Gateway.Home, "lab-web-metrics"), nil, 0600)
	last := map[string]uint64{}
	sawChatUpsert, sawOutside := false, false
	for i := 0; i < 32; i++ {
		action, key := "receipts", "message-000000"
		if i == 0 {
			action, key = "incoming", "web-readmodel-new"
		}
		if err := w.ControlAction("/messages", map[string]any{"action": action, "key": key}); err != nil {
			add("readmodel_sim_change", false)
			return
		}
		delivered := map[string]bool{}
		for tries := 0; tries < 40 && len(delivered) < 3; tries++ {
			frame, err := read()
			if err != nil {
				add("readmodel_live_frame", false)
				return
			}
			if frame["t"] == "error" {
				add("readmodel_live_error", false)
				return
			}
			if frame["t"] != "delta" {
				continue
			}
			sub, _ := frame["sub"].(string)
			if sub != "chats" && sub != "thread" && sub != "collection" {
				continue
			}
			cursor, _ := frame["cursor"].(map[string]any)
			seq, _ := cursor["sequence"].(float64)
			if uint64(seq) <= last[sub] {
				continue
			}
			last[sub] = uint64(seq)
			delivered[sub] = true
			if sub == "chats" {
				changes, _ := frame["changes"].([]any)
				for _, item := range changes {
					change, _ := item.(map[string]any)
					if change["op"] == "upsert" && change["row"] != nil {
						sawChatUpsert = true
					}
				}
			}
			if sub == "thread" {
				newer, _ := frame["newer"].(float64)
				sawOutside = sawOutside || newer == 2
			}
		}
		if len(delivered) != 3 {
			add("all_readmodel_subscriptions_advance", false)
			return
		}
	}
	add("chat_row_upsert", sawChatUpsert)
	add("thread_reports_newer_outside_window", sawOutside)
	data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "web-metrics.jsonl"))
	commits, writes := map[uint64]int64{}, map[uint64]int64{}
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
		} else if metric.At > writes[metric.Through] {
			writes[metric.Through] = metric.At
		}
	}
	samples := []float64{}
	for seq, at := range commits {
		if written := writes[seq]; written >= at {
			samples = append(samples, float64(written-at)/float64(time.Millisecond))
		}
	}
	e.Latency["journal_commit_to_last_readmodel_frame_written"] = stats(samples, 0)
	add("readmodel_latency_32_samples", err == nil && len(samples) == 32)

	add("readmodel_commit_to_frame_p95_20ms", len(samples) == 32 && stats(samples, 0).P95 <= 20)
	wsjson.Write(ctx, conn, map[string]any{"t": "sub", "id": "tail", "view": "thread", "args": map[string]any{"chat_id": chatID, "anchor": "latest", "limit": 3}})
	ready := false
	for tries := 0; tries < 12; tries++ {
		frame, err := read()
		if err != nil {
			break
		}
		if frame["t"] == "snap" && frame["sub"] == "tail" {
			data, _ := frame["data"].(map[string]any)
			ready = data["newer"] == float64(0)
			break
		}
	}
	add("latest_thread_at_tail", ready)
	err = w.ControlAction("/messages", map[string]any{"action": "incoming", "key": "web-readmodel-tail"})
	followed := false
	for tries := 0; tries < 20 && err == nil; tries++ {
		frame, readErr := read()
		if readErr != nil {
			break
		}
		if frame["t"] != "delta" || frame["sub"] != "tail" {
			continue
		}
		updates, _ := frame["changes"].([]any)
		for _, update := range updates {
			change, _ := update.(map[string]any)
			row, _ := change["row"].(map[string]any)
			data, _ := row["data"].(map[string]any)
			title, _ := data["title"].(string)
			if change["op"] == "upsert" && strings.Contains(title, "web-readmodel-tail") {
				followed = true
			}
		}
		if followed {
			break
		}
	}
	add("latest_thread_follows_new_inbound", followed)
}

type webZeroReader struct{}

func (webZeroReader) Read(p []byte) (int, error) { clear(p); return len(p), nil }

func runWebRateWindows(ctx context.Context, w *world.World, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("rate_authority", false)
		return
	}
	var clock atomic.Int64
	clock.Store(time.Now().UnixNano())
	now := func() time.Time { return time.Unix(0, clock.Load()) }
	fixture := httptest.NewServer(bridgeweb.Handler(bridgeweb.Deps{Authority: authority, RateClock: now}))
	defer fixture.Close()
	c := &webClient{ctx: ctx, origin: strings.Replace(fixture.URL, "127.0.0.1", "localhost", 1)}
	daily := true
	for minute := 0; minute < 10; minute++ {
		for count := 0; count < 10; count++ {
			resp, _, err := c.request("POST", "login/options", map[string]any{})
			daily = daily && err == nil && resp.StatusCode == 200
		}
		clock.Add(int64(time.Minute))
	}
	resp, _, err := c.request("POST", "login/options", map[string]any{})
	add("signin_daily_100_limit", daily && err == nil && resp.StatusCode == 429)
	clock.Add(int64(24 * time.Hour))
	resp, _, err = c.request("POST", "login/options", map[string]any{})
	add("signin_next_day_reset", err == nil && resp.StatusCode == 200)
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		add("api_session_fixture", false)
		return
	}
	c.cookie = &http.Cookie{Name: bridgeweb.SessionCookie, Value: token}
	api := true
	for count := 0; count < 50; count++ {
		resp, _, err = c.request("GET", "session", nil)
		api = api && err == nil && resp.StatusCode == 200
	}
	resp, _, err = c.request("GET", "session", nil)
	add("api_50_per_second", api && err == nil && resp.StatusCode == 429)
	clock.Add(int64(time.Second))
	resp, _, err = c.request("GET", "session", nil)
	add("api_second_reset", err == nil && resp.StatusCode == 200)
	handler := bridgeweb.Handler(bridgeweb.Deps{Authority: authority, RateClock: now})
	distributed := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		r.RemoteAddr = r.Header.Get("X-Lab-IP") + ":1234"
		handler.ServeHTTP(w, r)
	}))
	defer distributed.Close()
	failed := true
	request := func(ip, path string, input any) int {
		raw, _ := json.Marshal(input)
		origin := strings.Replace(distributed.URL, "127.0.0.1", "localhost", 1)
		req, _ := http.NewRequestWithContext(ctx, "POST", origin+"/web/v1/"+path, bytes.NewReader(raw))
		req.Header.Set("Origin", origin)
		req.Header.Set("X-Lab-IP", ip)
		resp, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0
		}
		io.Copy(io.Discard, resp.Body)
		resp.Body.Close()
		return resp.StatusCode
	}
	for i := 0; i < 10; i++ {
		failed = failed && request(fmt.Sprintf("192.0.2.%d", i+1), "enroll/options", map[string]any{"token": "invalid"}) == 401
	}
	add("global_failure_backoff_across_clients", failed && request("192.0.2.99", "login/options", map[string]any{}) == 429)
	clock.Add(int64(2 * time.Second))
	add("global_backoff_expires", request("192.0.2.99", "login/options", map[string]any{}) == 200)
}
