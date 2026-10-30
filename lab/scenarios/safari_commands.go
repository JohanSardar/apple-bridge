package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/publichttp"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runSafariCommands(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(n string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: n, Passed: ok})
		e.Passed = e.Passed && ok
	}
	if level == "sim" {
		output, readErr := os.ReadFile("native/.build/native-output-1")
		path := filepath.Join(strings.TrimSpace(string(output)), "SafariChecks")
		info, statErr := os.Stat(path)
		fresh := readErr == nil && statErr == nil
		if fresh {
			for _, dir := range []string{"SafariSource", "SafariChecks", "SourceSupport"} {
				walkErr := filepath.WalkDir(filepath.Join("native/Sources", dir), func(_ string, d os.DirEntry, e error) error {
					if e != nil {
						return e
					}
					if !d.IsDir() {
						f, e := d.Info()
						if e != nil {
							return e
						}
						if f.ModTime().After(info.ModTime()) {
							fresh = false
						}
					}
					return nil
				})
				if walkErr != nil {
					fresh = false
				}
			}
		}
		if !fresh {
			build := exec.CommandContext(ctx, "packaging/dev/build-native.sh")
			build.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_TARGET=SafariChecks")
			_, err := build.CombinedOutput()
			add("native_build", err == nil)
			if err != nil {
				return
			}
			output, readErr = os.ReadFile("native/.build/native-output-1")
			path = filepath.Join(strings.TrimSpace(string(output)), "SafariChecks")
		}
		add("native_output", readErr == nil)
		if readErr != nil {
			return
		}
		_, err := exec.CommandContext(ctx, path, "commands").CombinedOutput()
		add("native_fake_commands", err == nil)

	}
	scope := "safari:read,safari:write,system:read"
	ctx = world.WithOptions(ctx, world.Options{Scope: scope})
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewRealWithSetup(ctx, level, 0, world.NativeSetup{})
	} else {
		w, err = world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: scope})
	}
	add("world_started", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	w.Gateway.Token, err = w.ScopedToken(w.Gateway, scope)
	add("scope_granted", err == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	err = w.Await(20*time.Second, func() bool {
		var r contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &r) == nil
	})
	add("helper_ready", err == nil)
	if err != nil {
		return
	}
	command := func(op string, a any) contracts.CommandRecord {
		raw, _ := json.Marshal(a)
		var r contracts.CommandRecord
		err := client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &r)
		add(op, err == nil && r.State == contracts.CommandObserved && contracts.SafariCompletionEvidence(r.Evidence, r.RequestHash))
		if !e.Assertions[len(e.Assertions)-1].Passed {
			detail := string(r.State)
			if r.Error != nil {
				detail += ":" + string(r.Error.Code)
			}
			e.Assertions[len(e.Assertions)-1].Detail = detail
		}
		return r
	}
	if level == "real" && os.Getenv("APPLE_BRIDGE_SAFARI_RECOVER_RUNS") != "" {
		count, problem := recoverSafariRuns(ctx, w, os.Getenv("APPLE_BRIDGE_SAFARI_RECOVER_RUNS"))
		add("prior_run_owned_cleanup", problem == nil)
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		e.Metrics["recovered_fixture_count"] = count
		if problem != nil {
			return
		}
	}
	var cleanup *runCleanup
	var observeCleanup func() ([]cleanupItem, error)
	if level == "real" {
		observeCleanup = func() ([]cleanupItem, error) {
			return safariCleanupItems(context.Background(), w, e.RunID, "snapshot", cleanupItem{})
		}
		before, problem := observeCleanup()
		if problem != nil {
			e.Status = "limited"
			e.Passed = false
			e.Assertions = append(e.Assertions, Assertion{Name: "owned_cleanup_grants", Status: "limited", Detail: "needs Safari Automation and Accessibility grants"})
			return
		}
		cleanup, err = newRunCleanup(e.RunID, before)
		if cleanup != nil {
			cleanup.settle = 30 * time.Second
		}
		add("cleanup_baseline", err == nil)
		if err != nil {
			return
		}
		defer func() {
			proof, problem := cleanup.finish(observeCleanup, func(item cleanupItem) error {
				_, err := safariCleanupItems(context.Background(), w, e.RunID, "remove", item)
				return err
			})
			e.Metrics = map[string]any{"cleanup": proof}
			add("owned_cleanup_counts_match", problem == nil)
		}()
	}
	ownedCommand := func(op string) contracts.CommandRecord {
		value := command(op, map[string]string{"url": "https://example.com/?ab-test=" + e.RunID})
		if cleanup != nil {
			rows, problem := observeCleanup()
			add("created_identity_readback", problem == nil)
			if problem == nil {
				captured := false
				for _, item := range rows {
					if item.Tag == cleanup.tag {
						if _, exists := cleanup.created[item.ID]; !exists {
							problem = cleanup.capture(item)
							captured = captured || problem == nil
						}
					}
				}
				add("created_identity_captured", captured)
			}
		}
		return value
	}
	if s.ID == "SAF-READINGLIST-REAL" {
		ownedCommand("safari.add_reading_list")
		return
	}
	opened := ownedCommand("safari.open_url")
	var evidence struct {
		Readback struct {
			Tab string `json:"tab"`
		} `json:"readback"`
	}
	json.Unmarshal(opened.Evidence, &evidence)
	err = w.Await(10*time.Second, func() bool {
		rows, e := w.Records(w.Gateway)
		if e != nil {
			return false
		}
		for _, r := range rows {
			if r.Collection == "safari.tab" && !r.Deleted {
				return true
			}
		}
		return false
	})
	add("opened_tab_captured", err == nil)
	rejectClose := func(name, tab string) {
		raw, _ := json.Marshal(map[string]string{"tab": tab})
		var rejected contracts.CommandRecord
		err := client.Call(ctx, "safari.close_tab", contracts.Command{ID: contracts.NewCommandID(), Args: raw, WaitMS: 15000}, &rejected)
		add(name, err != nil && (strings.Contains(err.Error(), "invalid") || strings.Contains(err.Error(), "unsupported")))
	}
	rejectClose("owned_tab_close_rejected", evidence.Readback.Tab)
	rejectClose("foreign_tab_rejected", "foreign")
	if level == "sim" {
		var original struct {
			Readback struct {
				URL string `json:"url"`
			} `json:"readback"`
		}
		json.Unmarshal(opened.Evidence, &original)
		data, _ := json.Marshal(map[string]any{"window": 1, "index": 1, "title": "User replacement", "url": original.Readback.URL, "active": true})
		replacement := contracts.Observation{Kind: "upsert", Collection: "safari.tab", NativeKey: evidence.Readback.Tab, Data: data}
		add("same_url_replacement_submitted", w.ControlAction("/safari", []contracts.Observation{replacement}) == nil)
		err = w.Await(10*time.Second, func() bool {
			rows, e := w.Records(w.Gateway)
			if e != nil {
				return false
			}
			for _, r := range rows {
				if r.Collection == "safari.tab" && !r.Deleted && strings.Contains(string(r.Data), "User replacement") {
					return true
				}
			}
			return false
		})
		add("same_url_replacement_captured", err == nil)
		rejectClose("same_url_replacement_close_rejected", evidence.Readback.Tab)
	}
	ownedCommand("safari.add_reading_list")

	if level == "sim" {
		row := contracts.Observation{Kind: "upsert", Collection: "safari.tab", NativeKey: "synthetic", Data: json.RawMessage(`{"window":1,"index":1,"title":"Synthetic","url":"http://127.0.0.1/","active":true}`)}
		add("tab_capture_submitted", w.ControlAction("/safari", []contracts.Observation{row}) == nil)
		var tabID string
		err = w.Await(10*time.Second, func() bool {
			rows, e := w.Records(w.Gateway)
			if e != nil {
				return false
			}
			for _, r := range rows {
				if r.Collection == "safari.tab" && !r.Deleted && strings.Contains(string(r.Data), "http://127.0.0.1/") {
					tabID = r.ID
					return true
				}
			}
			return false
		})
		add("tab_captured", err == nil)
		args, _ := json.Marshal(map[string]string{"tab": tabID})
		var result contracts.QueryResult
		err = client.Call(ctx, "safari.page_text", contracts.Query{Args: args, DeadlineMS: 4000}, &result)
		add("gateway_page_text_private_rejected", err != nil && (strings.Contains(err.Error(), "invalid") || strings.Contains(err.Error(), "unsupported")))
	}
	if level == "sim" {
		safariSSRF(ctx, add)
	}
	return
}
func safariSSRF(ctx context.Context, add func(string, bool)) {
	for _, ip := range []string{"127.0.0.1", "10.0.0.1", "169.254.169.254", "::1", "::ffff:127.0.0.1", "100.64.0.1"} {
		_, err := operations.FetchSafariPage(ctx, &publichttp.Guard{}, "http://"+net.JoinHostPort(ip, "80")+"/")
		add("private_ip_rejected", err != nil)
	}
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/redirect" {
			http.Redirect(w, r, "http://127.0.0.1/", 302)
			return
		}
		w.Header().Set("Content-Type", "text/html")
		if r.URL.Path == "/large" {
			w.Write([]byte(strings.Repeat("x", 262145)))
			return
		}
		w.Write([]byte("<p>Public &amp; text</p><script>secret</script>"))
	}))
	defer server.Close()
	calls := 0
	dials := 0
	guard := &publichttp.Guard{Resolve: func(context.Context, string) ([]net.IPAddr, error) {
		calls++
		ip := "93.184.216.34"
		if calls > 1 {
			ip = "127.0.0.1"
		}
		return []net.IPAddr{{IP: net.ParseIP(ip)}}, nil
	}, DialContext: func(c context.Context, n, a string) (net.Conn, error) {
		dials++
		return (&net.Dialer{}).DialContext(c, n, server.Listener.Addr().String())
	}}
	_, err := operations.FetchSafariPage(ctx, guard, "http://public.example/")
	add("rebinding_rejected_before_dial", err != nil && calls == 2 && dials == 0)
	guard.Resolve = func(_ context.Context, host string) ([]net.IPAddr, error) {
		ip := "93.184.216.34"
		if host == "127.0.0.1" {
			ip = host
		}
		return []net.IPAddr{{IP: net.ParseIP(ip)}}, nil
	}
	raw, err := operations.FetchSafariPage(ctx, guard, "http://public.example/")
	add("public_text_extracted", err == nil && strings.Contains(string(raw), "Public") && !strings.Contains(string(raw), "secret"))
	before := dials
	_, err = operations.FetchSafariPage(ctx, guard, "http://public.example/redirect")
	add("redirect_loopback_rejected", err != nil && dials == before+1)
	_, err = operations.FetchSafariPage(ctx, guard, "http://public.example/large")
	add("size_cap", err != nil)
}
