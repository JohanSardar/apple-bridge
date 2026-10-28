package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runQueries(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewReal(ctx, level, 0)
	} else {
		w, err = world.New(ctx, level, 1, 0)
	}
	add("world_started", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	client := scripted.Client{Endpoint: w.Gateway}
	if level == "replica" {
		client.Endpoint = w.Replicas[0]
	}
	q := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"value":"lab"}`), DeadlineMS: 1000}
	call := func(name string, q contracts.Query) (contracts.QueryResult, error) {
		var out contracts.QueryResult
		err := client.Call(ctx, name, q, &out)
		return out, err
	}
	if level == "real" {
		q.Args = json.RawMessage(`{}`)
		var readinessErr error
		err = w.Await(15*time.Second, func() bool {
			q.ID = contracts.NewID("qry")
			var out contracts.QueryResult
			out, readinessErr = call("helper.version", q)
			return readinessErr == nil && out.ID == q.ID && json.Valid(out.ResultJSON)
		})
		add("helper_query_ready", err == nil)
		if err != nil {
			var problem *contracts.Error
			code := "other"
			if errors.As(readinessErr, &problem) {
				switch problem.Code {
				case "unavailable", "deadline", "unsupported", "invalid", "forbidden":
					code = problem.Code
				}
			}
			e.Metrics = map[string]any{"readiness_error": code}
			return
		}
	}
	switch s.ID {
	case "CORE-QUERY-ROUNDTRIP", "CORE-QUERY-HELPER", "CORE-QUERY-REPLICA":
		name := "test.echo"
		budget := 200 * time.Millisecond
		if level == "real" {
			name = "helper.version"
			q.Args = json.RawMessage(`{}`)
			budget = time.Second
		}
		var samples []float64
		resultsOK, valuesOK := true, true
		for i := 0; i < 10; i++ {
			q.ID = contracts.NewID("qry")
			start := time.Now()
			out, err := call(name, q)
			samples = append(samples, float64(time.Since(start))/float64(time.Millisecond))
			resultsOK = resultsOK && err == nil && out.ID == q.ID && json.Valid(out.ResultJSON)
			if name == "helper.version" {
				var version struct {
					Version     string `json:"version"`
					Protocol    string `json:"protocol"`
					HelperBuild string `json:"helper_build"`
				}
				decodeErr := json.Unmarshal(out.ResultJSON, &version)
				valuesOK = valuesOK && decodeErr == nil && version.Version != "" && version.HelperBuild != "" && version.Protocol == contracts.IPCProtocol
			}
			if name == "test.echo" {
				valuesOK = valuesOK && string(out.ResultJSON) == string(q.Args)
			}
		}
		add("ten_query_results", resultsOK && len(samples) == 10)
		if name == "helper.version" {
			add("helper_build_identified", valuesOK)
		} else {
			add("echo_equal", valuesOK)
		}
		maxMS := float64(0)
		for _, v := range samples {
			if v > maxMS {
				maxMS = v
			}
		}
		e.Latency["query"] = stats(samples, 0)
		add("latency_budget", maxMS < float64(budget)/float64(time.Millisecond))
		if level == "sim" {
			out, _ := call(name, q)
			client.Protocol = "mcp"
			mcp, err := call(name, q)
			add("mcp_parity", err == nil && string(out.ResultJSON) == string(mcp.ResultJSON))
			raw, readErr := os.ReadFile(filepath.Join(w.Gateway.Home, "commands-audit.jsonl"))
			auditOK, found := readErr == nil, false
			for _, line := range strings.Split(strings.TrimSpace(string(raw)), "\n") {
				var row map[string]any
				if json.Unmarshal([]byte(line), &row) == nil && row["operation"] == "test.echo" {
					found = true
					auditOK = auditOK && len(row) == 4 && row["argument_bytes"] != nil && row["result_bytes"] != nil && row["blob_bytes"] != nil
				}
			}
			add("audit_names_and_sizes_only", auditOK && found)
			q.DeadlineMS = -1
			_, err = call(name, q)
			add("invalid_rejected", err != nil)
		}
		if level == "replica" {
			longQuery := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"ms":25000}`), DeadlineMS: 30000}
			longStart := time.Now()
			longResult, longErr := call("test.sleep", longQuery)
			elapsed := time.Since(longStart)
			add("replica_work_exceeds_twenty_seconds", longErr == nil && longResult.ID == longQuery.ID && elapsed > 20*time.Second && elapsed < 30*time.Second)
			e.Latency["long_query"] = stats([]float64{float64(elapsed) / float64(time.Millisecond)}, 0)
			feedWorld, feedErr := world.New(world.WithOptions(ctx, world.Options{GatewayDials: true}), level, 1, 0)
			add("query_feed_world_started", feedErr == nil)
			if feedErr == nil {
				feedClient := scripted.Client{Endpoint: feedWorld.Replicas[0]}
				longQuery.ID = contracts.NewID("qry")
				start := time.Now()
				var result contracts.QueryResult
				err := feedClient.Call(ctx, "test.sleep", longQuery, &result)
				elapsed := time.Since(start)
				add("feed_work_exceeds_twenty_seconds", err == nil && result.ID == longQuery.ID && elapsed > 20*time.Second && elapsed < 30*time.Second)
				e.Latency["feed_long_query"] = stats([]float64{float64(elapsed) / float64(time.Millisecond)}, 0)
				feedWorld.Close()
			}
			deadlineQuery := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"ms":5000}`), DeadlineMS: 100}
			start := time.Now()
			_, deadlineErr := call("test.sleep", deadlineQuery)
			var problem *contracts.Error
			add("replica_deadline_bounded", errors.As(deadlineErr, &problem) && problem.Code == "deadline" && time.Since(start) < 200*time.Millisecond)
			assertQueryCancellation(ctx, w, client, add)
			w.StopGateway()
			start = time.Now()
			_, err = call(name, q)
			add("offline_unavailable", errors.Is(err, contracts.ErrUnavailable) && time.Since(start) < time.Second)
		}
	case "CORE-QUERY-DEADLINE":
		q.Args = json.RawMessage(`{"ms":5000}`)
		q.DeadlineMS = 100
		start := time.Now()
		_, err = call("test.sleep", q)
		var problem *contracts.Error
		add("deadline_bounded", errors.As(err, &problem) && problem.Code == "deadline" && time.Since(start) < 200*time.Millisecond)
		for _, protocol := range []string{"rest", "mcp"} {
			client.Protocol = protocol
			assertQueryCancellation(ctx, w, client, func(name string, ok bool) { add(protocol+"_"+name, ok) })
		}
	case "CORE-QUERY-SCOPE":
		token, err := w.ScopedToken(w.Gateway, "calendar:read")
		add("scoped_token", err == nil)
		client.Endpoint.Token = token
		_, err = call("test.echo", q)
		add("missing_grant_rejected", errors.Is(err, contracts.ErrForbidden))
		client.Endpoint = w.Gateway
		_, err = call("test.echo", q)
		add("read_allowed", err == nil)
		authority, err := trust.Open(w.Gateway.Home)
		add("agent_authority", err == nil)
		if err != nil {
			return
		}
		_, token, err = authority.CreateAgent("Bridge Test query read", trust.AgentRead, []contracts.Grant{"reminders:read"})
		add("read_agent_created", err == nil)
		client.Endpoint.Token = token
		_, err = call("test.echo", q)
		add("read_agent_allowed", err == nil)
		_, token, err = authority.CreateAgent("Bridge Test query approve", trust.AgentApprove, []contracts.Grant{"reminders:write"})
		add("approve_agent_created", err == nil)
		client.Endpoint.Token = token
		var pending contracts.CommandRecord
		err = client.Call(ctx, "reminders.create_list", contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{"title":"[ab-test query] approval"}`)}, &pending)
		add("effect_requires_approval", err == nil && pending.State == contracts.CommandAwaitingApproval)
	case "CORE-QUERY-BLOB":
		q.Args = json.RawMessage(`{"size":5242880}`)
		q.DeadlineMS = 10000
		out, err := call("test.blob", q)
		add("blob_result", err == nil && len(out.Blobs) == 1)
		if err != nil || len(out.Blobs) != 1 {
			return
		}
		b := out.Blobs[0]
		read := func(endpoint world.Endpoint, rng string) (int, []byte) {
			req, _ := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/blobs/"+b.Digest+"?record="+q.ID, nil)
			req.Host = endpoint.Host
			req.Header.Set("Authorization", "Bearer "+endpoint.Token)
			if rng != "" {
				req.Header.Set("Range", rng)
			}
			resp, err := http.DefaultClient.Do(req)
			if err != nil {
				return 0, nil
			}
			defer resp.Body.Close()
			data, _ := io.ReadAll(resp.Body)
			return resp.StatusCode, data
		}
		status, data := read(client.Endpoint, "")
		sum := sha256.Sum256(data)
		add("digest_matches", status == 200 && len(data) == 5242880 && hex.EncodeToString(sum[:]) == b.Digest)
		status, data = read(client.Endpoint, "bytes=11-30")
		add("range_read", status == 206 && len(data) == 20 && data[0] == 11)
		scoped := client.Endpoint
		scoped.Token, _ = w.ScopedToken(client.Endpoint, "calendar:read")
		status, _ = read(scoped, "")
		add("blob_scope", status == 403)
	}
	return
}

func assertQueryCancellation(ctx context.Context, w *world.World, client scripted.Client, add func(string, bool)) {
	q := contracts.Query{ID: contracts.NewID("qry"), Args: json.RawMessage(`{"ms":5000}`), DeadlineMS: 10000}
	cancelctx, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan error, 1)
	go func() { done <- client.Call(cancelctx, "test.sleep", q, nil) }()
	err := w.Await(2*time.Second, func() bool { effects, err := w.Effects(); return err == nil && effects[q.ID] == 1 })
	add("helper_work_started", err == nil)
	cancel()
	select {
	case err = <-done:
		add("client_cancelled", err != nil)
	case <-time.After(time.Second):
		add("client_cancelled", false)
	}
	err = w.Await(time.Second, func() bool { effects, err := w.Effects(); return err == nil && effects[q.ID] == 0 })
	add("helper_work_stopped", err == nil)
}
