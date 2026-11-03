package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"flag"
	"fmt"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"io"
	"net/http"
	"os"
	"regexp"
	"time"

	"database/sql"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"path/filepath"
)

func request(e world.Endpoint, path string, body []byte) ([]byte, error) {
	req, err := http.NewRequest("POST", e.URL+path, bytes.NewReader(body))
	if err != nil {
		return nil, err
	}
	req.Host = e.Host
	req.Header.Set("Authorization", "Bearer "+e.Token)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	req.Header.Set("MCP-Protocol-Version", "2025-03-26")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	return io.ReadAll(resp.Body)
}
func run() error {
	if len(os.Args) > 1 && os.Args[1] == "simapple" {
		f := flag.NewFlagSet("simapple", flag.ContinueOnError)
		socket := f.String("socket", "", "")
		secret := f.String("secret-file", "", "")
		control := f.String("control", "", "")
		if err := f.Parse(os.Args[2:]); err != nil {
			return err
		}
		return simapple.Run(context.Background(), simapple.Config{Socket: *socket, SecretFile: *secret, Control: *control})
	}

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Minute)
	defer cancel()
	ctx = world.WithImplementations(ctx, world.Implementations{Gateway: "go", Replica: "rust", Replicas: []string{"go", "rust"}, RustBinary: os.Args[1], LocalReplicas: true, Prebuilt: true})
	w, err := world.New(ctx, "replica", 2, 100, world.BlobOptions{Scope: "calendar:read,calendar:write,reminders:read,reminders:write,contacts:read,notes:read,messages:read,health:read,iphone:read,system:read"})
	if err != nil {
		return err
	}
	defer func() {
		b, _ := os.ReadFile(filepath.Join(w.Dir, "replica1.log"))
		fmt.Printf("FEED_LOG %.2000s\n", b)
		w.Close()
	}()
	fmt.Println("HOME", w.Dir)
	if err = w.SeedMessages(20); err != nil {
		return err
	}
	if err = w.ControlAction("/blob-fixture", map[string]any{"size": 262144}); err != nil {
		return err
	}
	if err = w.FlushCaptures(30 * time.Second); err != nil {
		return err
	}
	err = w.Await(30*time.Second, func() bool {
		a, ca, ea := w.RecordsAt(w.Replicas[0])
		b, cb, eb := w.RecordsAt(w.Replicas[1])
		return ea == nil && eb == nil && len(a) > 0 && len(a) == len(b) && ca == cb
	})
	if err != nil {
		return err
	}
	if err = w.Await(15*time.Second, func() bool {
		for _, endpoint := range w.Replicas {
			var result contracts.ReadResult
			if _, err := world.Request(ctx, endpoint, "/v1/op/sources.status", struct{}{}, &result); err != nil || result.Replica == nil || len(result.Replica.Blobs.Entries) == 0 {
				return false
			}
			for _, entry := range result.Replica.Blobs.Entries {
				if entry.State != "present" {
					return false
				}
			}
		}
		return true
	}); err != nil {
		return fmt.Errorf("blob parity fixture: %w", err)
	}
	cases := []struct{ name, input string }{
		{"records.query", `{}`}, {"sources.status", `{}`}, {"replica.status", `{}`},
		{"iphone.devices", `{}`}, {"health.summary", `{"period":"day","date":"2026-11-02","time_zone":"UTC"}`},
		{"calendar.agenda", `{"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z"}`}, {"search", `{"text":"seed"}`},
		{"contacts.lookup", `{"q":"Synthetic"}`}, {"contacts.list", `{}`},
		{"messages.chats", `{}`}, {"messages.handles", `{"ids":[]}`},
		{"messages.search", `{"text":"needle"}`},
		{"notes.list", `{}`}, {"reminders.lists", `{"time_zone":"UTC"}`},
		{"reminders.items", `{"time_zone":"UTC"}`},
		{"calendar.range", `{"start":"2026-01-01T00:00:00Z","end":"2026-01-02T00:00:00Z","time_zone":"UTC"}`},
	}
	recordsForGet, _, err := w.RecordsAt(w.Replicas[1])
	if err != nil || len(recordsForGet) == 0 {
		return fmt.Errorf("record fixture missing: %v", err)
	}
	recordInput, _ := json.Marshal(map[string]string{"collection": recordsForGet[0].Collection, "id": recordsForGet[0].ID})
	cases = append(cases, struct{ name, input string }{"records.get", string(recordInput)}, struct{ name, input string }{"messages.thread", `{"chat_id":"cht_01ARZ3NDEKTSV4RRFFQ69G5FAV"}`}, struct{ name, input string }{"messages.collection", `{"chat_id":"cht_01ARZ3NDEKTSV4RRFFQ69G5FAV","kind":"photos"}`}, struct{ name, input string }{"commands.list", `{}`}, struct{ name, input string }{"commands.status", `{"command_id":"cmd_absent_parity_probe"}`})
	failures := 0
	for _, c := range cases {
		a, err := request(w.Replicas[0], "/v1/op/"+c.name, []byte(c.input))
		if err != nil {
			return err
		}
		b, err := request(w.Replicas[1], "/v1/op/"+c.name, []byte(c.input))
		if err != nil {
			return err
		}
		if c.name == "sources.status" || c.name == "replica.status" {
			if !timing(c.name, a) || !timing(c.name, b) {
				return fmt.Errorf("live timing invalid for %s", c.name)
			}
			fmt.Printf("TIMING %s valid=true\n", c.name)
		}
		same := bytes.Equal(normalize(c.name, a), normalize(c.name, b))
		fmt.Printf("REST %s equal=%t\n", c.name, same)
		if !same {
			failures++
			fmt.Printf("GO %.500sRUST %.500s\n", a, b)
		}
		params := map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": map[string]any{"name": tool(c.name), "arguments": json.RawMessage(c.input)}}
		body, _ := json.Marshal(params)
		a, err = request(w.Replicas[0], "/mcp", body)
		if err != nil {
			return err
		}
		b, err = request(w.Replicas[1], "/mcp", body)
		if err != nil {
			return err
		}
		var ga, gb map[string]any
		if json.Unmarshal(a, &ga) != nil || json.Unmarshal(b, &gb) != nil {
			return fmt.Errorf("invalid MCP: %s %s", a, b)
		}
		ra, _ := ga["result"].(map[string]any)
		rb, _ := gb["result"].(map[string]any)
		if ra != nil {
			delete(ra, "_meta")
			normalizeMCP(c.name, ra)
		}
		if rb != nil {
			delete(rb, "_meta")
			normalizeMCP(c.name, rb)
		}
		aa, _ := json.Marshal(ga)
		bb, _ := json.Marshal(gb)
		same = bytes.Equal(aa, bb) && rawMCP(c.name, a, b)
		fmt.Printf("MCP %s equal=%t\n", c.name, same)
		if !same {
			failures++
			fmt.Printf("GO %.500sRUST %.500s\n", a, b)
		}
	}
	records, _, err := w.RecordsAt(w.Replicas[1])
	if err != nil {
		return err
	}
	list := ""
	for _, r := range records {
		if r.Collection == "reminders.list" {
			list = r.ID
			break
		}
	}
	command, _ := json.Marshal(map[string]any{"command_id": "cmd_phase3_runtime_probe", "args": map[string]any{"list_id": list, "title": "[ab-test] Rust runtime forwarding"}, "wait_ms": 0})
	raw, err := request(w.Replicas[1], "/v1/op/reminders.create", command)
	if err != nil {
		return err
	}
	var submitted map[string]any
	if json.Unmarshal(raw, &submitted) != nil {
		return fmt.Errorf("invalid command response")
	}
	inspection, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "ledger.db")+"?mode=ro")
	if err != nil {
		return err
	}
	defer inspection.Close()
	var disposition string
	err = w.Await(30*time.Second, func() bool {
		var payload []byte
		e := inspection.QueryRowContext(ctx, "SELECT payload FROM commands WHERE id=?", "cmd_phase3_runtime_probe").Scan(&payload)
		var r map[string]any
		if e == nil && json.Unmarshal(payload, &r) == nil {
			disposition, _ = r["disposition"].(string)
		}
		return disposition != ""
	})
	if err != nil {
		return err
	}
	var authoritative map[string]any
	var status []byte
	_ = w.Await(20*time.Second, func() bool {
		status, err = request(w.Replicas[1], "/v1/op/commands.status", []byte(`{"command_id":"cmd_phase3_runtime_probe"}`))
		if err != nil {
			return false
		}
		_ = json.Unmarshal(status, &authoritative)
		return authoritative["disposition"] != nil
	})
	same := disposition != "" && disposition == authoritative["disposition"]
	fmt.Printf("COMMAND forwarded=%t replica_disposition=%v gateway_disposition=%v\n", same, authoritative["disposition"], disposition)
	if !same {
		failures++
		fmt.Printf("COMMAND submitted=%s status=%s\n", raw, status)
	}
	for _, protocol := range []string{"rest", "mcp"} {
		if !cancellation(ctx, w, protocol) {
			failures++
		}
	}
	if err = offlineBlob(ctx, w); err != nil {
		return err
	}
	if failures > 0 {
		return fmt.Errorf("%d byte-parity failures", failures)
	}
	return nil
}
func tool(s string) string {
	b := []byte(s)
	for i := range b {
		if b[i] == '.' {
			b[i] = '_'
		}
	}
	return string(b)
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func normalize(name string, raw []byte) []byte {
	if name != "sources.status" && name != "replica.status" {
		return raw
	}
	raw = regexp.MustCompile(`"last_heartbeat":"[^"]*"`).ReplaceAll(raw, []byte(`"last_heartbeat":"LIVE"`))
	raw = regexp.MustCompile(`"staleness_ms":[0-9]+`).ReplaceAll(raw, []byte(`"staleness_ms":0`))
	if name == "replica.status" {
		raw = regexp.MustCompile(`"observed_at":"[^"]*"`).ReplaceAll(raw, []byte(`"observed_at":"LIVE"`))
	}
	return raw
}
func normalizeMCP(name string, result map[string]any) {
	content, _ := result["content"].([]any)
	for _, item := range content {
		if object, ok := item.(map[string]any); ok {
			if text, ok := object["text"].(string); ok {
				object["text"] = string(normalize(name, []byte(text)))
			}
		}
	}
	if structured, ok := result["structuredContent"].(map[string]any); ok {
		if status, ok := structured["replica"].(map[string]any); ok {
			status["last_heartbeat"] = "LIVE"
			status["staleness_ms"] = 0
		}
		if name == "replica.status" {
			if records, ok := structured["records"].([]any); ok {
				for _, item := range records {
					if record, ok := item.(map[string]any); ok {
						record["observed_at"] = "LIVE"
						if status, ok := record["data"].(map[string]any); ok {
							status["last_heartbeat"] = "LIVE"
							status["staleness_ms"] = 0
						}
					}
				}
			}
		}
	}
}

func timing(name string, raw []byte) bool {
	type liveStatus struct {
		Heartbeat string `json:"last_heartbeat"`
		Staleness int64  `json:"staleness_ms"`
	}
	var value struct {
		Replica liveStatus `json:"replica"`
		Records []struct {
			Collection string     `json:"collection"`
			ObservedAt string     `json:"observed_at"`
			Data       liveStatus `json:"data"`
		} `json:"records"`
	}
	if json.Unmarshal(raw, &value) != nil {
		return false
	}
	recent := func(text string) bool {
		timestamp, err := time.Parse(time.RFC3339Nano, text)
		age := time.Since(timestamp)
		return err == nil && age >= 0 && age < 15*time.Second
	}
	valid := func(status liveStatus) bool {
		return recent(status.Heartbeat) && status.Staleness >= 0 && status.Staleness < 15000
	}
	if !valid(value.Replica) {
		return false
	}
	if name == "replica.status" {
		return len(value.Records) == 1 && value.Records[0].Collection == "system.replica" && recent(value.Records[0].ObservedAt) && valid(value.Records[0].Data)
	}
	return true
}

func cancellation(ctx context.Context, w *world.World, protocol string) bool {
	query := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"ms":5000}`), DeadlineMS: 10000}
	bounded, cancelQuery := context.WithCancel(ctx)
	done := make(chan error, 1)
	go func() {
		done <- (scripted.Client{Endpoint: w.Replicas[1], Protocol: protocol}).Call(bounded, "test.sleep", query, nil)
	}()
	started := w.Await(2*time.Second, func() bool { effects, e := w.Effects(); return e == nil && effects[query.ID] == 1 }) == nil
	cancelQuery()
	cancelled := false
	select {
	case e := <-done:
		cancelled = e != nil
	case <-time.After(time.Second):
	}
	stopped := w.Await(time.Second, func() bool { effects, e := w.Effects(); return e == nil && effects[query.ID] == 0 }) == nil
	fmt.Printf("CANCELLATION %s started=%t client_cancelled=%t helper_stopped=%t\n", protocol, started, cancelled, stopped)
	return started && cancelled && stopped
}

func offlineBlob(ctx context.Context, w *world.World) error {
	query := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"size":262144}`), DeadlineMS: 10000}
	client := scripted.Client{Endpoint: w.Replicas[1], Protocol: "mcp"}
	var result contracts.QueryResult
	if err := client.Call(ctx, "test.blob", query, &result); err != nil {
		return err
	}
	if len(result.Blobs) != 1 {
		return fmt.Errorf("query blob result missing")
	}
	blob := result.Blobs[0]
	read := func(rng string) ([]byte, error) {
		e := w.Replicas[1]
		req, err := http.NewRequestWithContext(ctx, "GET", e.URL+"/v1/blobs/"+blob.Digest+"?record="+query.ID, nil)
		if err != nil {
			return nil, err
		}
		req.Host = e.Host
		req.Header.Set("Authorization", "Bearer "+e.Token)
		if rng != "" {
			req.Header.Set("Range", rng)
		}
		response, err := http.DefaultClient.Do(req)
		if err != nil {
			return nil, err
		}
		defer response.Body.Close()
		data, err := io.ReadAll(response.Body)
		if err != nil {
			return nil, err
		}
		if response.StatusCode != 200 && response.StatusCode != 206 {
			return nil, fmt.Errorf("query blob HTTP %d", response.StatusCode)
		}
		return data, nil
	}
	first, err := read("")
	if err != nil {
		return err
	}
	if fmt.Sprintf("%x", sha256.Sum256(first)) != blob.Digest {
		return fmt.Errorf("query blob digest mismatch")
	}
	for _, test := range []struct {
		header   string
		expected []byte
	}{{"bytes=11-30", first[11:31]}, {"bytes=11-", first[11:]}, {"bytes=-20", first[len(first)-20:]}} {
		part, err := read(test.header)
		if err != nil || !bytes.Equal(part, test.expected) {
			return fmt.Errorf("query blob range %s differs: %v", test.header, err)
		}
	}
	fmt.Println("QUERY_BLOB ranges=3/3")
	w.StopGateway()
	cached, err := read("")
	if err != nil {
		return err
	}
	equal := bytes.Equal(first, cached)
	fmt.Printf("QUERY_BLOB cached_offline=%t sha256_valid=true\n", equal)
	if !equal {
		return fmt.Errorf("offline query blob differs")
	}
	return nil
}

func rawMCP(name string, a, b []byte) bool {
	type envelope struct {
		Result struct {
			Structured json.RawMessage `json:"structuredContent"`
			Content    []struct {
				Text string `json:"text"`
			} `json:"content"`
			Meta map[string]float64 `json:"_meta"`
		} `json:"result"`
	}
	var left, right envelope
	if json.Unmarshal(a, &left) != nil || json.Unmarshal(b, &right) != nil {
		return false
	}
	if len(left.Result.Content) != 1 || len(right.Result.Content) != 1 {
		return false
	}
	for _, payload := range []envelope{left, right} {
		if (name == "sources.status" || name == "replica.status") && !timing(name, payload.Result.Structured) {
			return false
		}
		if !bytes.Equal(normalize(name, payload.Result.Structured), normalize(name, []byte(payload.Result.Content[0].Text))) {
			return false
		}
		for key, value := range payload.Result.Meta {
			if key != "bridge/read_model_ms" || value < 0 {
				return false
			}
		}
	}
	return (left.Result.Meta == nil) == (right.Result.Meta == nil) && bytes.Equal(normalize(name, left.Result.Structured), normalize(name, right.Result.Structured))
}
