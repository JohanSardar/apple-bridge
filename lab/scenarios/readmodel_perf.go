package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/demo"
)

func readmodelTableDigest(ctx context.Context, db *sql.DB, table, order string) (string, error) {
	rows, err := db.QueryContext(ctx, "SELECT * FROM "+table+" ORDER BY "+order)
	if err != nil {
		return "", err
	}
	defer rows.Close()
	columns, err := rows.Columns()
	if err != nil {
		return "", err
	}
	digest := sha256.New()
	encoder := json.NewEncoder(digest)
	for rows.Next() {
		values := make([]any, len(columns))
		pointers := make([]any, len(columns))
		for i := range values {
			pointers[i] = &values[i]
		}
		if err = rows.Scan(pointers...); err != nil {
			return "", err
		}
		if err = encoder.Encode(values); err != nil {
			return "", err
		}
	}
	return hex.EncodeToString(digest.Sum(nil)), rows.Err()
}

func readmodelDigests(ctx context.Context, db *sql.DB) (map[string]string, error) {
	result := map[string]string{}
	for _, table := range []string{"msg_by_chat", "reaction_by_message", "attachment_by_message", "activity_by_chat", "chat_summary"} {
		digest, err := readmodelTableDigest(ctx, db, table, "id")
		if err != nil {
			return nil, err
		}
		result[table] = digest
	}
	return result, nil
}

func runReadmodelUpdatePerf(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: string(bytes.TrimSpace(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}, Metrics: map[string]any{}, Conditions: map[string]string{"dataset": "synthetic huge demo", "transport": "State capture transaction API", "capture": "five captures of 100 existing messages", "budget": "1000 ms p95 transaction guard derived from R-LIVE; not end-to-end certification"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	home, err := os.MkdirTemp("", "ab-readmodel-perf-")
	if err != nil {
		add("private_store", false)
		return e
	}
	defer os.RemoveAll(home)
	fixture := os.Getenv("APPLE_BRIDGE_READMODEL_FIXTURE")
	if fixture != "" {
		err = exec.CommandContext(ctx, "cp", "-c", fixture, filepath.Join(home, "state.db")).Run()
		add("clone_closed_fixture", err == nil)
		if err != nil {
			return e
		}
		e.Conditions["seed"] = "APFS clone of closed synthetic fixture; seed timing measured separately"
	}
	store, err := state.Open(home)
	if err != nil {
		add("open_store", false)
		return e
	}
	defer store.Close()
	db, err := sql.Open("sqlite", "file:"+filepath.Join(home, "state.db")+"?_pragma=busy_timeout(5000)")
	if err != nil {
		add("inspection_connection", false)
		return e
	}
	defer db.Close()
	checkpoints := map[contracts.Source]string{}
	generations := map[contracts.Source]string{}
	if fixture == "" {
		generated := demo.Generate(demo.Options{Seed: 1, Huge: true})
		grouped := map[contracts.Source][]contracts.Observation{}
		for _, observation := range generated.Records {
			source := contracts.CollectionSource(observation.Collection)
			grouped[source] = append(grouped[source], observation)
		}
		seedCtx, cancel := context.WithTimeout(ctx, 2*time.Minute)
		start := time.Now()
		for _, source := range contracts.Sources {
			records := grouped[source]
			generations[source] = "readmodel-demo"
			for len(records) > 0 {
				count := min(2048, len(records))
				observations := records[:count]
				observed := time.Date(2026, 1, 1, 0, 0, 0, 0, time.UTC)
				for i := range observations {
					observations[i].ObservedAt = observed
				}
				next := fmt.Sprintf("seed-%d", len(records))
				_, err = store.CommitCapture(seedCtx, contracts.Capture{Source: source, Generation: generations[source], Predecessor: checkpoints[source], Checkpoint: next, Observations: observations})
				if err != nil {
					break
				}
				checkpoints[source] = next
				records = records[count:]
			}
			if err != nil {
				break
			}
		}
		elapsed := float64(time.Since(start)) / float64(time.Millisecond)
		cancel()
		e.Latency["huge_demo_seed_commit"] = stats([]float64{elapsed}, 0)
		add("huge_demo_seed_within_dataset_120_second_budget", err == nil && elapsed < 120000)
		if err != nil {
			return e
		}
	} else {
		rows, problem := db.QueryContext(ctx, "SELECT source,generation,checkpoint FROM checkpoints")
		if problem != nil {
			add("fixture_checkpoints", false)
			return e
		}
		for rows.Next() {
			var source contracts.Source
			var generation, checkpoint string
			if problem = rows.Scan(&source, &generation, &checkpoint); problem != nil {
				break
			}
			generations[source], checkpoints[source] = generation, checkpoint
		}
		problem = errors.Join(problem, rows.Err(), rows.Close())
		if problem != nil {
			add("fixture_checkpoints", false)
			return e
		}
	}
	synthetic := generations[contracts.Messages] == "fts-demo" || generations[contracts.Messages] == "readmodel-demo"
	add("synthetic_fixture_generation", synthetic)
	if !synthetic {
		return e
	}
	for {
		_, err = store.Read(ctx, trust.Visibility{contracts.Messages: true}, contracts.ReadQuery{Text: "readmodelDefinitelyAbsent", Limit: 1})
		if err == nil {
			break
		}
		if !errors.Is(err, contracts.ErrUnavailable) {
			add("search_ready", false)
			return e
		}
		select {
		case <-ctx.Done():
			add("search_ready", false)
			return e
		case <-time.After(10 * time.Millisecond):
		}
	}
	var messages int
	err = db.QueryRowContext(ctx, "SELECT count(*) FROM records WHERE collection='messages.message' AND deleted=0").Scan(&messages)
	e.Metrics["persisted_messages"] = messages
	add("fixed_104900_message_fixture", err == nil && messages == 104900)
	if err != nil || messages != 104900 {
		return e
	}
	fixtureDigest, err := readmodelTableDigest(ctx, db, "records", "collection,id")
	if err != nil {
		add("fixture_digest", false)
		return e
	}
	before, err := readmodelDigests(ctx, db)
	if err != nil {
		add("initial_readmodels_digest", false)
		return e
	}
	e.Metrics["fixture_sha256"] = fixtureDigest
	e.Metrics["readmodels_before"] = before
	type target struct {
		key    string
		record contracts.Record
	}
	rows, err := db.QueryContext(ctx, `SELECT a.native_key,r.payload FROM records r JOIN aliases a ON a.record_id=r.id AND a.collection=r.collection WHERE r.collection='messages.message' AND r.deleted=0 AND a.generation=? ORDER BY r.id LIMIT 100`, generations[contracts.Messages])
	if err != nil {
		add("existing_targets", false)
		return e
	}
	targets := []target{}
	for rows.Next() {
		var t target
		var payload []byte
		if err = rows.Scan(&t.key, &payload); err != nil {
			break
		}
		if err = json.Unmarshal(payload, &t.record); err != nil {
			break
		}
		targets = append(targets, t)
	}
	err = errors.Join(err, rows.Err(), rows.Close())
	add("exactly_100_existing_update_targets", err == nil && len(targets) == 100)
	if err != nil || len(targets) != 100 {
		return e
	}
	timings := []float64{}
	for round := 0; round < 5; round++ {
		observations := []contracts.Observation{}
		for _, t := range targets {
			var data map[string]any
			if err = json.Unmarshal(t.record.Data, &data); err != nil {
				break
			}
			data["text"] = fmt.Sprintf("[ab-test] readmodel round %d", round)
			if round%2 == 0 {
				data["read_at"] = "2030-01-01T00:00:00Z"
			} else {
				data["read_at"] = nil
			}
			raw, problem := json.Marshal(data)
			if problem != nil {
				err = problem
				break
			}
			observations = append(observations, contracts.Observation{Kind: "upsert", Collection: t.record.Collection, NativeKey: t.key, ObservedAt: time.Date(2030, 1, 1, 0, 0, round, 0, time.UTC), Data: raw})
		}
		if err != nil {
			add("capture_encoding", false)
			return e
		}
		next := fmt.Sprintf("readmodel-perf-%d", round)
		start := time.Now()
		result, problem := store.CommitCapture(ctx, contracts.Capture{Source: contracts.Messages, Generation: generations[contracts.Messages], Predecessor: checkpoints[contracts.Messages], Checkpoint: next, Observations: observations})
		elapsed := float64(time.Since(start)) / float64(time.Millisecond)
		add("capture_commits_exactly_100_changes", problem == nil && result.Through-result.After == 100)
		if problem != nil {
			return e
		}
		timings = append(timings, elapsed)
		checkpoints[contracts.Messages] = next
	}
	e.Latency["capture_100_commit"] = stats(timings, 0)
	add("capture_commit_p95_under_one_second", e.Latency["capture_100_commit"].P95 < 1000)
	after, err := readmodelDigests(ctx, db)
	if err != nil {
		add("final_readmodels_digest", false)
		return e
	}
	e.Metrics["readmodels_after"] = after
	checks := map[string]string{
		"message_rows_and_payloads_match_canonical":    `SELECT (SELECT count(*) FROM records WHERE collection='messages.message' AND deleted=0)=(SELECT count(*) FROM msg_by_chat) AND NOT EXISTS(SELECT 1 FROM msg_by_chat m LEFT JOIN records r ON r.collection='messages.message' AND r.id=m.id WHERE r.id IS NULL OR r.deleted!=0 OR r.payload!=m.payload)`,
		"reaction_rows_and_payloads_match_canonical":   `SELECT (SELECT count(*) FROM records WHERE collection='messages.reaction' AND deleted=0)=(SELECT count(*) FROM reaction_by_message) AND NOT EXISTS(SELECT 1 FROM reaction_by_message m LEFT JOIN records r ON r.collection='messages.reaction' AND r.id=m.id WHERE r.id IS NULL OR r.deleted!=0 OR r.payload!=m.payload)`,
		"attachment_rows_and_payloads_match_canonical": `SELECT (SELECT count(*) FROM records WHERE collection='messages.attachment' AND deleted=0)=(SELECT count(*) FROM attachment_by_message) AND NOT EXISTS(SELECT 1 FROM attachment_by_message m LEFT JOIN records r ON r.collection='messages.attachment' AND r.id=m.id WHERE r.id IS NULL OR r.deleted!=0 OR r.payload!=m.payload)`,
		"chat_summaries_match_index_aggregates":        `SELECT NOT EXISTS(SELECT 1 FROM chat_summary s WHERE s.unread_count!=coalesce((SELECT sum(m.unread) FROM msg_by_chat m WHERE m.chat_id=s.id),0) OR s.last_activity_at!=coalesce((SELECT at FROM activity_by_chat a WHERE a.chat_id=s.id ORDER BY at DESC,id DESC LIMIT 1),''))`,
		"activity_rows_match_messages_and_reactions":   `SELECT (SELECT count(*) FROM activity_by_chat)=(SELECT count(*) FROM msg_by_chat)+(SELECT count(*) FROM reaction_by_message r JOIN msg_by_chat m ON m.id=r.message_id) AND NOT EXISTS(SELECT 1 FROM activity_by_chat a LEFT JOIN msg_by_chat m ON m.id=a.message_id WHERE m.id IS NULL OR m.chat_id!=a.chat_id OR (a.id=a.message_id AND a.at!=m.sent_at) OR (a.id!=a.message_id AND NOT EXISTS(SELECT 1 FROM reaction_by_message r WHERE r.id=a.id AND r.message_id=a.message_id AND r.at=a.at)))`,
	}
	for name, query := range checks {
		var correct bool
		err = db.QueryRowContext(ctx, query).Scan(&correct)
		add(name, err == nil && correct)
	}
	if baseline := os.Getenv("APPLE_BRIDGE_READMODEL_BASELINE"); baseline != "" {
		var previous Evidence
		raw, problem := os.ReadFile(baseline)
		if problem == nil {
			problem = json.Unmarshal(raw, &previous)
		}
		rawBefore, _ := json.Marshal(e.Metrics["readmodels_before"])
		rawAfter, _ := json.Marshal(e.Metrics["readmodels_after"])
		var normalizedBefore, normalizedAfter any
		json.Unmarshal(rawBefore, &normalizedBefore)
		json.Unmarshal(rawAfter, &normalizedAfter)
		add("identical_cloned_canonical_fixture", problem == nil && previous.Metrics["fixture_sha256"] == fixtureDigest)
		add("all_five_readmodel_tables_byte_equal_to_baseline", problem == nil && reflect.DeepEqual(previous.Metrics["readmodels_before"], normalizedBefore) && reflect.DeepEqual(previous.Metrics["readmodels_after"], normalizedAfter))
	}
	e.Passed = true
	for _, a := range e.Assertions {
		if !a.Passed {
			e.Passed = false
		}
	}
	return e
}

func readmodelAliasContract(ctx context.Context) bool {
	home, err := os.MkdirTemp("", "ab-readmodel-alias-")
	if err != nil {
		return false
	}
	defer os.RemoveAll(home)
	store, err := state.Open(home)
	if err != nil {
		return false
	}
	defer store.Close()
	sequence := 0
	put := func(generation, previous, checkpoint, key, text string, aliases bool) error {
		sequence++
		at := time.Date(2031, 1, 1, 0, 0, sequence, 0, time.UTC)
		records := []contracts.Observation{
			rmObservation("messages.message", key, map[string]any{"chat_id": rmRef("messages.chat", key), "sender_handle_id": rmRef("messages.handle", key), "kind": "message", "text": text, "sent_at": at, "is_from_me": false}),
			rmObservation("messages.chat", key, map[string]any{"display_name": "[ab-test] alias chat", "participants": []any{}}),
			rmObservation("messages.handle", key, map[string]any{"address": "probe@example.com", "service": "iMessage"}),
		}
		for i := range records {
			records[i].ObservedAt = at
			if aliases {
				records[i].ScopedAlias = "readmodel/" + records[i].Collection
			}
		}
		_, err := store.CommitCapture(ctx, contracts.Capture{Source: contracts.Messages, Generation: generation, Predecessor: previous, Checkpoint: checkpoint, Observations: records})
		return err
	}
	read := func(collection string) ([]contracts.Record, error) {
		result, err := store.Read(ctx, trust.Visibility{contracts.Messages: true}, contracts.ReadQuery{Collection: collection, Limit: 20})
		return result.Records, err
	}
	if put("first", "", "one", "same", "[ab-test] original", true) != nil {
		return false
	}
	messages, err := read("messages.message")
	if err != nil || len(messages) != 1 {
		return false
	}
	chats, err := read("messages.chat")
	if err != nil || len(chats) != 1 {
		return false
	}
	handles, err := read("messages.handle")
	if err != nil || len(handles) != 1 {
		return false
	}
	messageID, chatID, handleID := messages[0].ID, chats[0].ID, handles[0].ID
	if messageID == chatID || chatID == handleID || messageID == handleID {
		return false
	}
	data := rmData(messages[0])
	if data["chat_id"] != chatID || data["sender_handle_id"] != handleID {
		return false
	}
	if put("second", "", "two", "renamed", "[ab-test] resumed", true) != nil {
		return false
	}
	messages, err = read("messages.message")
	if err != nil || len(messages) != 1 || messages[0].ID != messageID || messages[0].Version != 2 {
		return false
	}
	data = rmData(messages[0])
	if data["chat_id"] != chatID || data["sender_handle_id"] != handleID {
		return false
	}
	at := time.Date(2031, 1, 1, 0, 0, 3, 0, time.UTC)
	independent := rmObservation("messages.message", "same", map[string]any{"chat_id": rmRef("messages.chat", "renamed"), "sender_handle_id": rmRef("messages.handle", "renamed"), "kind": "message", "text": "[ab-test] independent", "sent_at": at, "is_from_me": false})
	independent.ObservedAt = at
	if _, err = store.CommitCapture(ctx, contracts.Capture{Source: contracts.Messages, Generation: "second", Predecessor: "two", Checkpoint: "three", Observations: []contracts.Observation{independent}}); err != nil {
		return false
	}
	messages, err = read("messages.message")
	if err != nil || len(messages) != 2 {
		return false
	}
	distinct := false
	for _, record := range messages {
		if rmData(record)["text"] == "[ab-test] independent" {
			distinct = record.ID != messageID
		}
	}
	if !distinct {
		return false
	}
	alias := contracts.Observation{Kind: "alias", Collection: "messages.message", NativeKey: "alias-only", ScopedAlias: "readmodel/messages.message", ObservedAt: at.Add(time.Second)}
	updated := independent
	updated.NativeKey = "alias-only"
	updated.ScopedAlias = alias.ScopedAlias
	updated.ObservedAt = alias.ObservedAt
	fields := rmData(messages[0])
	fields["chat_id"] = rmRef("messages.chat", "renamed")
	fields["sender_handle_id"] = rmRef("messages.handle", "renamed")
	fields["text"] = "[ab-test] alias update"
	updated.Data = mustJSON(fields)
	if _, err = store.CommitCapture(ctx, contracts.Capture{Source: contracts.Messages, Generation: "second", Predecessor: "three", Checkpoint: "four", Observations: []contracts.Observation{alias, updated}}); err != nil {
		return false
	}
	messages, err = read("messages.message")
	if err != nil || len(messages) != 2 {
		return false
	}
	for _, record := range messages {
		if rmData(record)["text"] == "[ab-test] alias update" {
			return record.ID == messageID && record.Version == 3
		}
	}
	return false
}

func runCaptureAliases(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	passed := readmodelAliasContract(ctx)
	return Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: string(bytes.TrimSpace(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Passed: passed, Assertions: []Assertion{{Name: "capture_alias_scopes_generations_and_forward_references", Passed: passed}}, Conditions: map[string]string{"dataset": "synthetic contacts-free Messages identity fixture", "transport": "State capture and query APIs"}}
}
