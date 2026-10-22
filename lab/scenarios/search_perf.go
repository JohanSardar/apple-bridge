package scenarios

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/frontdoor"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/demo"
)

func runSearchUpdatePerf(ctx context.Context, scenario Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: scenario.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}, Metrics: map[string]any{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	home, err := os.MkdirTemp("", "ab-fts-perf-")
	if err != nil {
		add("private_store", false)
		return e
	}
	defer os.RemoveAll(home)
	fixture := os.Getenv("APPLE_BRIDGE_FTS_FIXTURE")
	if fixture != "" {
		if err = exec.CommandContext(ctx, "cp", "-c", fixture, filepath.Join(home, "state.db")).Run(); err != nil {
			add("clone_seeded_fixture", false)
			return e
		}
	}
	opened := time.Now()
	store, err := state.Open(home)
	if err != nil {
		add("state_open", false)
		return e
	}
	e.Latency["fixture_startup"] = stats([]float64{float64(time.Since(opened)) / float64(time.Millisecond)}, 0)
	defer func() {
		if store != nil {
			store.Close()
		}
	}()
	database, err := sql.Open("sqlite", "file:"+filepath.Join(home, "state.db")+"?_pragma=busy_timeout(5000)")
	if err != nil {
		add("maintenance_connection", false)
		return e
	}
	defer database.Close()
	visibility := trust.Visibility{}
	for _, source := range contracts.Sources {
		visibility[source] = true
	}
	checkpoints := map[contracts.Source]string{}
	captureNumber := 0
	var lastCommit state.CommitResult
	commit := func(source contracts.Source, observations []contracts.Observation) error {
		captureNumber++
		checkpoint := fmt.Sprint(captureNumber)
		now := time.Now().UTC()
		for i := range observations {
			observations[i].ObservedAt = now
			observations[i].Revalidated = true
		}
		collections := []string{}
		for _, observation := range observations {
			if !slices.Contains(collections, observation.Collection) {
				collections = append(collections, observation.Collection)
			}
		}
		var problem error
		lastCommit, problem = store.CommitCapture(ctx, contracts.Capture{Source: source, Generation: "fts-demo", Predecessor: checkpoints[source], Checkpoint: checkpoint, Observations: observations, ScanID: checkpoint, Scope: collections, ScanStartedAt: now, CompletionEvidence: "complete"})
		if problem == nil {
			checkpoints[source] = checkpoint
		}
		return problem
	}
	dataset := demo.Generate(demo.Options{Seed: 1, Huge: true})
	add("unchanged_huge_demo_104900_messages", dataset.Counts["messages.message"] == 104900)
	grouped := map[contracts.Source][]contracts.Observation{}
	targets := []contracts.Observation{}
	for _, observation := range dataset.Records {
		source := contracts.CollectionSource(observation.Collection)
		grouped[source] = append(grouped[source], observation)
		if observation.Collection == "messages.message" && strings.HasPrefix(observation.NativeKey, "demo-message-39-") && len(targets) < 100 {
			targets = append(targets, observation)
		}
	}
	seedStarted := time.Now()
	if fixture == "" {
		for _, source := range contracts.Sources {
			records := grouped[source]
			for len(records) > 0 {
				size := min(2048, len(records))
				if err = commit(source, records[:size]); err != nil {
					add("huge_demo_captured", false)
					e.Assertions[len(e.Assertions)-1].Detail = err.Error()
					return e
				}
				records = records[size:]
			}
		}
	} else {
		rows, problem := database.QueryContext(ctx, "SELECT source,checkpoint FROM checkpoints")
		if problem != nil {
			add("fixture_checkpoints", false)
			return e
		}
		for rows.Next() {
			var source contracts.Source
			var checkpoint string
			if problem = rows.Scan(&source, &checkpoint); problem != nil {
				break
			}
			checkpoints[source] = checkpoint
			var value int
			fmt.Sscan(checkpoint, &value)
			captureNumber = max(captureNumber, value)
		}
		problem = errors.Join(problem, rows.Err(), rows.Close())
		if problem != nil {
			add("fixture_checkpoints", false)
			return e
		}
	}
	for {
		_, problem := store.Read(ctx, visibility, contracts.ReadQuery{Text: "ftsDefinitelyAbsent", Limit: 1})
		if !errors.Is(problem, contracts.ErrUnavailable) {
			if problem != nil {
				add("initial_search_ready", false)
				return e
			}
			break
		}
		select {
		case <-ctx.Done():
			add("initial_search_ready", false)
			return e
		case <-time.After(10 * time.Millisecond):
		}
	}
	e.Latency["fixture_until_search_ready"] = stats([]float64{float64(time.Since(opened)) / float64(time.Millisecond)}, 0)
	e.Latency["huge_demo_seed"] = stats([]float64{float64(time.Since(seedStarted)) / float64(time.Millisecond)}, 0)
	var messages int
	err = database.QueryRowContext(ctx, "SELECT count(*) FROM records WHERE collection='messages.message' AND deleted=0").Scan(&messages)
	e.Metrics["persisted_messages"] = messages
	add("persisted_104900_messages", err == nil && messages == 104900)
	queries := []string{"imaginary garden", "paper boats", "prototype", "Synthetic Studio", "ftsDefinitelyAbsent"}
	matches := func(query string) ([]string, error) {
		ids := []string{}
		after := ""
		for {
			result, problem := store.Read(ctx, visibility, contracts.ReadQuery{Text: query, After: after, Limit: 1000})
			if problem != nil {
				return nil, problem
			}
			for _, record := range result.Records {
				ids = append(ids, record.Collection+"/"+record.ID)
			}
			if result.Next == "" {
				return ids, nil
			}
			after = result.Next
		}
	}
	identities := map[string]string{}
	rows, err := database.QueryContext(ctx, "SELECT collection,record_id,native_key FROM aliases WHERE generation='fts-demo'")
	if err != nil {
		add("fixture_identities", false)
		return e
	}
	for rows.Next() {
		var collection, id, key string
		if err = rows.Scan(&collection, &id, &key); err != nil {
			break
		}
		identities[collection+"/"+id] = collection + "/" + key
	}
	err = errors.Join(err, rows.Err(), rows.Close())
	if err != nil {
		add("fixture_identities", false)
		return e
	}
	snapshot := func() (map[string][]string, error) {
		out := map[string][]string{}
		for _, query := range queries {
			ids, problem := matches(query)
			if problem != nil {
				return nil, problem
			}
			for i, id := range ids {
				ids[i] = identities[id]
			}
			slices.Sort(ids)
			out[query] = ids
		}
		return out, nil
	}
	before, err := snapshot()
	if err != nil {
		add("baseline_search_sets", false)
		return e
	}
	for query, ids := range before {
		e.Metrics["query_matches_"+query] = len(ids)
		e.Metrics["query_set_sha256_"+query] = fmt.Sprintf("%x", sha256.Sum256([]byte(strings.Join(ids, "\n"))))
	}
	updates := func(iteration int) []contracts.Observation {
		out := make([]contracts.Observation, len(targets))
		for i, original := range targets {
			out[i] = original
			var data map[string]any
			json.Unmarshal(original.Data, &data)
			data["read_at"] = demo.Epoch.Add(time.Duration(iteration+1) * time.Second)
			out[i].Data, _ = json.Marshal(data)
		}
		return out
	}
	timings := []float64{}
	allHundred := true
	for iteration := 0; iteration < 5; iteration++ {
		started := time.Now()
		if err = commit(contracts.Messages, updates(iteration)); err != nil {
			add("hundred_record_capture", false)
			e.Assertions[len(e.Assertions)-1].Detail = err.Error()
			return e
		}
		timings = append(timings, float64(time.Since(started))/float64(time.Millisecond))
		allHundred = allHundred && lastCommit.Through-lastCommit.After == 100
		e.Latency["hundred_record_capture_commit"] = stats(timings, 0)
	}
	e.Latency["hundred_record_capture_commit"] = stats(timings, 0)
	add("five_samples_commit_exactly_100_updates", len(timings) == 5 && allHundred)
	add("hundred_record_commit_p95_under_one_second", len(targets) == 100 && e.Latency["hundred_record_capture_commit"].P95 < 1000)
	after, err := snapshot()
	add("fixed_search_sets_equal_after_updates", err == nil && reflect.DeepEqual(before, after))
	var targetID string
	err = database.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE source='messages' AND generation='fts-demo' AND collection='messages.message' AND native_key=?", targets[0].NativeKey).Scan(&targetID)
	if err != nil {
		add("target_identity", false)
		return e
	}
	store.Close()
	store = nil
	_, err = database.ExecContext(ctx, "DROP TRIGGER IF EXISTS records_search_insert; DROP TRIGGER IF EXISTS records_search_update; DROP TRIGGER IF EXISTS records_search_delete; DROP TABLE IF EXISTS records_fts_map; UPDATE meta SET value='2' WHERE key='fts_version'")
	if err != nil {
		add("legacy_v2_fixture", false)
		return e
	}
	started := time.Now()
	store, err = state.Open(home)
	e.Latency["migration_startup"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	if err != nil {
		add("migration_open", false)
		return e
	}
	_, problem := store.Read(ctx, visibility, contracts.ReadQuery{Text: "prototype", Limit: 1})
	pending := errors.Is(problem, contracts.ErrUnavailable)
	add("search_unavailable_during_rebuild", pending)
	searchPreparingClients(ctx, home, store, add)
	_, problem = store.Read(ctx, visibility, contracts.ReadQuery{Limit: 1})
	add("ordinary_reads_available_during_rebuild", problem == nil)
	if pending {
		store.Close()
		store = nil
		started = time.Now()
		store, err = state.Open(home)
		if err != nil {
			add("migration_restart", false)
			return e
		}
		e.Latency["migration_resume_startup"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
		_, problem = store.Read(ctx, visibility, contracts.ReadQuery{Text: "prototype", Limit: 1})
		add("restart_resumes_pending_rebuild", errors.Is(problem, contracts.ErrUnavailable))
	} else {
		add("restart_resumes_pending_rebuild", false)
	}
	add("migration_startup_under_500ms", e.Latency["migration_startup"].Max < 500)
	captureStarted := time.Now()
	if err = commit(contracts.Messages, updates(30)); err != nil {
		add("capture_progress_during_rebuild", false)
		return e
	}
	e.Latency["capture_during_rebuild"] = stats([]float64{float64(time.Since(captureStarted)) / float64(time.Millisecond)}, 0)
	add("capture_progress_during_rebuild", true)
	add("capture_during_rebuild_under_one_second", e.Latency["capture_during_rebuild"].Max < 1000)
	for {
		_, problem = store.Read(ctx, visibility, contracts.ReadQuery{Text: "ftsDefinitelyAbsent", Limit: 1})
		if !errors.Is(problem, contracts.ErrUnavailable) {
			break
		}
		select {
		case <-ctx.Done():
			add("rebuild_completed", false)
			return e
		case <-time.After(10 * time.Millisecond):
		}
	}
	e.Latency["migration_until_search_ready"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	var version string
	err = database.QueryRowContext(ctx, "SELECT value FROM meta WHERE key='fts_version'").Scan(&version)
	add("fts_version_three", problem == nil && err == nil && version == "3")
	after, err = snapshot()
	add("fixed_search_sets_equal_after_rebuild", err == nil && reflect.DeepEqual(before, after))
	var stableID int64
	database.QueryRowContext(ctx, "SELECT fts_rowid FROM records_fts_map WHERE collection='messages.message' AND id=?", targetID).Scan(&stableID)
	store.Close()
	store = nil
	_, err = database.ExecContext(ctx, "UPDATE records SET rowid=(SELECT max(rowid)+1024 FROM records) WHERE collection='messages.message' AND id=?", targetID)
	if err == nil {
		_, err = database.ExecContext(ctx, "VACUUM")
	}
	add("vacuum_completed", err == nil)
	store, err = state.Open(home)
	if err != nil {
		add("vacuum_reopen", false)
		return e
	}
	if err = commit(contracts.Messages, updates(31)); err != nil {
		add("capture_after_vacuum", false)
		return e
	}
	var stableAfter int64
	err = database.QueryRowContext(ctx, "SELECT fts_rowid FROM records_fts_map WHERE collection='messages.message' AND id=?", targetID).Scan(&stableAfter)
	add("stable_fts_key_after_record_rowid_change_and_vacuum", err == nil && stableID > 0 && stableID == stableAfter)
	after, err = snapshot()
	add("fixed_search_sets_equal_after_vacuum_update", err == nil && reflect.DeepEqual(before, after))
	changed := updates(32)[:1]
	var data map[string]any
	json.Unmarshal(changed[0].Data, &data)
	data["text"] = "ftsUniqueReplacement"
	changed[0].Data, _ = json.Marshal(data)
	err = commit(contracts.Messages, changed)
	ids, problem := matches("ftsUniqueReplacement")
	add("changed_text_indexed_once", err == nil && problem == nil && reflect.DeepEqual(ids, []string{"messages.message/" + targetID}))
	changed[0].Kind = "absent"
	err = commit(contracts.Messages, changed)
	ids, problem = matches("ftsUniqueReplacement")
	add("delete_removes_search_match", err == nil && problem == nil && len(ids) == 0)
	calendar := contracts.Observation{Kind: "upsert", Collection: "calendar.event", NativeKey: "fts-evict", Data: json.RawMessage(`{"title":"ftsUniqueEviction"}`)}
	err = commit(contracts.Calendar, []contracts.Observation{calendar})
	ids, problem = matches("ftsUniqueEviction")
	add("calendar_search_match", err == nil && problem == nil && len(ids) == 1)
	calendar.Kind = "evict"
	err = commit(contracts.Calendar, []contracts.Observation{calendar})
	ids, problem = matches("ftsUniqueEviction")
	add("eviction_removes_search_match", err == nil && problem == nil && len(ids) == 0)
	delete(visibility, contracts.Calendar)
	err = store.Restrict(ctx, visibility)
	var orphans int
	problem = database.QueryRowContext(ctx, "SELECT count(*) FROM records_fts f LEFT JOIN records r ON r.collection=f.collection AND r.id=f.id WHERE r.id IS NULL OR r.deleted!=0").Scan(&orphans)
	add("deleted_and_evicted_fts_rows_absent", err == nil && problem == nil && orphans == 0)
	problem = database.QueryRowContext(ctx, "SELECT count(*) FROM records_fts_map m LEFT JOIN records r ON r.collection=m.collection AND r.id=m.id WHERE r.id IS NULL").Scan(&orphans)
	add("hard_delete_removes_mapping", problem == nil && orphans == 0)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}

func searchPreparingClients(ctx context.Context, home string, store *state.Store, add func(string, bool)) {
	previous, wasSet := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
	os.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	defer func() {
		if wasSet {
			os.Setenv("APPLE_BRIDGE_SECRET_STORE", previous)
		} else {
			os.Unsetenv("APPLE_BRIDGE_SECRET_STORE")
		}
	}()
	authority, err := trust.Open(filepath.Join(home, "client-authority"))
	if err != nil {
		add("search_client_authority", false)
		return
	}
	token, err := authority.CreateToken([]contracts.Grant{"messages:read"})
	if err != nil {
		add("search_client_token", false)
		return
	}
	handler := frontdoor.Handler(frontdoor.Deps{Authority: authority, Address: "127.0.0.1:12345", Operations: &operations.Service{Authority: authority, State: store}}, contracts.Operations)
	call := func(path, body string) *httptest.ResponseRecorder {
		request := httptest.NewRequest(http.MethodPost, "http://localhost:12345"+path, strings.NewReader(body)).WithContext(ctx)
		request.Header.Set("Authorization", "Bearer "+token)
		request.Header.Set("Content-Type", "application/json")
		request.Header.Set("Accept", "application/json, text/event-stream")
		response := httptest.NewRecorder()
		handler.ServeHTTP(response, request)
		return response
	}
	rest := call("/v1/op/search", `{"text":"prototype","limit":1}`)
	var failure struct {
		Error *contracts.Error `json:"error"`
	}
	err = json.Unmarshal(rest.Body.Bytes(), &failure)
	add("search_rest_preparing_is_retryable_error_not_empty", err == nil && rest.Code == http.StatusServiceUnavailable && failure.Error != nil && failure.Error.Code == "unavailable" && failure.Error.Retryable)
	reply := call("/mcp", `{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search","arguments":{"text":"prototype","limit":1}}}`)
	var result struct {
		Result struct {
			IsError           bool `json:"isError"`
			StructuredContent struct {
				Error *contracts.Error `json:"error"`
			} `json:"structuredContent"`
		} `json:"result"`
	}
	err = json.Unmarshal(reply.Body.Bytes(), &result)
	problem := result.Result.StructuredContent.Error
	add("search_mcp_preparing_is_retryable_error_not_empty", err == nil && reply.Code == http.StatusOK && result.Result.IsError && problem != nil && problem.Code == "unavailable" && problem.Retryable)
}
