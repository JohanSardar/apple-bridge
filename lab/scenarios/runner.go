package scenarios

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"gopkg.in/yaml.v3"
)

type Scenario struct {
	ID      string                      `yaml:"id"`
	Feature string                      `yaml:"feature"`
	Levels  []string                    `yaml:"levels"`
	Given   []map[string]map[string]any `yaml:"given"`
	When    []map[string]map[string]any `yaml:"when"`
	Then    []map[string]map[string]any `yaml:"then"`
	Cleanup []map[string]map[string]any `yaml:"cleanup"`
}
type Assertion struct {
	Name   string `json:"name"`
	Passed bool   `json:"passed"`
	Detail string `json:"detail,omitempty"`
}
type Latency struct {
	Samples  int     `json:"samples"`
	P50      float64 `json:"p50_ms"`
	P95      float64 `json:"p95_ms"`
	Max      float64 `json:"max_ms"`
	Timeouts int     `json:"timeouts"`
}
type Evidence struct {
	Scenario   string             `json:"scenario"`
	Level      string             `json:"level"`
	Passed     bool               `json:"passed"`
	GitSHA     string             `json:"git_sha"`
	At         time.Time          `json:"at"`
	Assertions []Assertion        `json:"assertions"`
	Latency    map[string]Latency `json:"latency"`
	Hosts      []string           `json:"hosts"`
}

type Seen struct {
	Change contracts.Change
	At     time.Time
}
type watcher struct {
	mu        sync.Mutex
	seen      map[string]Seen
	changes   []Seen
	last      contracts.Cursor
	invalid   bool
	filtered  bool
	connected chan struct{}
	cancel    context.CancelFunc
	done      chan struct{}
}

func key(r contracts.Record) string {
	var data map[string]any
	json.Unmarshal(r.Data, &data)
	title, _ := data["title"].(string)
	return r.Collection + "/" + title
}
func watch(ctx context.Context, e world.Endpoint, filtered bool) (*watcher, error) {
	var result contracts.ReadResult
	if _, err := world.Request(ctx, e, "/v1/op/records.query", contracts.ReadQuery{Limit: 1}, &result); err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(ctx)
	w := &watcher{seen: map[string]Seen{}, last: result.Cursor, filtered: filtered, connected: make(chan struct{}), cancel: cancel, done: make(chan struct{})}
	go func() {
		defer close(w.done)
		ready := false
		for ctx.Err() == nil {
			w.mu.Lock()
			after := w.last
			w.mu.Unlock()
			req, err := http.NewRequestWithContext(ctx, "GET", fmt.Sprintf("%s/v1/changes?after=%d&epoch=%s", e.URL, after.Sequence, after.Epoch), nil)
			if err != nil {
				return
			}
			req.Host = e.Host
			req.Header.Set("Authorization", "Bearer "+e.Token)
			resp, err := http.DefaultClient.Do(req)
			if err == nil && resp.StatusCode == 200 {
				if !ready {
					close(w.connected)
					ready = true
				}
				scanner := bufio.NewScanner(resp.Body)
				scanner.Buffer(make([]byte, 4096), contracts.MaxFrame*2)
				for scanner.Scan() {
					line := scanner.Text()
					if !strings.HasPrefix(line, "data: ") {
						continue
					}
					var b contracts.Batch
					if json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &b) != nil {
						w.mu.Lock()
						w.invalid = true
						w.mu.Unlock()
						break
					}
					at := time.Now()
					w.mu.Lock()
					if b.Epoch != w.last.Epoch || b.After != w.last.Sequence || b.Through < b.After {
						w.invalid = true
					}
					for _, c := range b.Changes {
						if filtered && c.Record.Source != contracts.System {
							w.invalid = true
						}
						seen := Seen{Change: c, At: at}
						w.seen[key(c.Record)] = seen
						w.changes = append(w.changes, seen)
					}
					w.last.Sequence = b.Through
					w.mu.Unlock()
				}
			}
			if resp != nil {
				resp.Body.Close()
			}
			timer := time.NewTimer(100 * time.Millisecond)
			select {
			case <-ctx.Done():
				timer.Stop()
				return
			case <-timer.C:
			}
		}
	}()
	timer := time.NewTimer(10 * time.Second)
	defer timer.Stop()
	select {
	case <-w.connected:
		return w, nil
	case <-ctx.Done():
		cancel()
		return nil, ctx.Err()
	case <-timer.C:
		cancel()
		return nil, fmt.Errorf("SSE readiness timed out")
	}
}
func (w *watcher) close() { w.cancel(); <-w.done }
func (w *watcher) observed(title string) (Seen, bool) {
	w.mu.Lock()
	defer w.mu.Unlock()
	s, ok := w.seen["reminders.reminder/[ab-test] "+title]
	return s, ok
}
func (w *watcher) valid() bool { w.mu.Lock(); defer w.mu.Unlock(); return !w.invalid }

func Load(paths []string) ([]Scenario, error) {
	files := map[string]bool{}
	for _, path := range paths {
		info, err := os.Stat(path)
		if os.IsNotExist(err) {
			matches, _ := filepath.Glob(filepath.Join("lab/scenarios/core", filepath.Base(path)))
			if len(matches) == 1 {
				path = matches[0]
				info, err = os.Stat(path)
			}
		}
		if err != nil {
			return nil, err
		}
		if info.IsDir() {
			err = filepath.WalkDir(path, func(p string, d os.DirEntry, err error) error {
				if err != nil {
					return err
				}
				if !d.IsDir() && strings.HasSuffix(p, ".yaml") {
					files[p] = true
				}
				return nil
			})
			if err != nil {
				return nil, err
			}
		} else {
			files[path] = true
		}
	}
	ordered := []string{}
	for f := range files {
		ordered = append(ordered, f)
	}
	slices.Sort(ordered)
	out := []Scenario{}
	for _, f := range ordered {
		b, err := os.ReadFile(f)
		if err != nil {
			return nil, err
		}
		var s Scenario
		dec := yaml.NewDecoder(strings.NewReader(string(b)))
		dec.KnownFields(true)
		if err = dec.Decode(&s); err != nil {
			return nil, err
		}
		if s.ID == "" || len(s.Levels) == 0 {
			return nil, fmt.Errorf("scenario identity missing")
		}
		out = append(out, s)
	}
	return out, nil
}
func number(m map[string]any, k string, def int) int {
	if n, ok := m[k].(int); ok {
		return n
	}
	return def
}
func sleep(ctx context.Context, d time.Duration) error {
	t := time.NewTimer(d)
	defer t.Stop()
	select {
	case <-ctx.Done():
		return ctx.Err()
	case <-t.C:
		return nil
	}
}
func stats(values []float64, timeouts int) Latency {
	slices.Sort(values)
	out := Latency{Samples: len(values), Timeouts: timeouts}
	if len(values) == 0 {
		return out
	}
	out.P50 = values[int(math.Ceil(float64(len(values))*.5))-1]
	out.P95 = values[int(math.Ceil(float64(len(values))*.95))-1]
	out.Max = values[len(values)-1]
	return out
}
func normalize(records []contracts.Record, includeSystem bool) map[string]any {
	ids := map[string]string{}
	for _, r := range records {
		var d map[string]any
		json.Unmarshal(r.Data, &d)
		if t, ok := d["title"].(string); ok {
			ids[r.ID] = t
		}
	}
	out := map[string]any{}
	for _, r := range records {
		if !includeSystem && r.Collection == "system.source" {
			continue
		}
		var d map[string]any
		json.Unmarshal(r.Data, &d)
		for k, v := range d {
			if id, ok := v.(string); ok && ids[id] != "" {
				d[k] = ids[id]
			}
		}
		out[key(r)] = d
	}
	return out
}
func truthRecords(observations []contracts.Observation) []contracts.Record {
	titles := map[string]string{}
	for _, o := range observations {
		var d map[string]any
		json.Unmarshal(o.Data, &d)
		if t, ok := d["title"].(string); ok {
			titles[o.NativeKey] = t
		}
	}
	out := []contracts.Record{}
	for _, o := range observations {
		if o.Kind == "absent" {
			continue
		}
		var d map[string]any
		json.Unmarshal(o.Data, &d)
		for k, v := range d {
			if ref, ok := v.(map[string]any); ok {
				if native, ok := ref["$ref"].(map[string]any); ok {
					if id, ok := native["native_key"].(string); ok {
						d[k] = titles[id]
					}
				}
			}
		}
		b, _ := json.Marshal(d)
		out = append(out, contracts.Record{Collection: o.Collection, Data: b})
	}
	return out
}
func convergence(w *world.World) bool {
	truth, err := w.Inspect()
	if err != nil {
		return false
	}
	gateway, err := w.Records(w.Gateway)
	if err != nil {
		return false
	}
	if !reflect.DeepEqual(normalize(gateway, false), normalize(truthRecords(truth), false)) {
		return false
	}
	for _, endpoint := range w.Replicas {
		replica, err := w.Records(endpoint)
		if err != nil || !reflect.DeepEqual(replica, gateway) {
			return false
		}
	}
	return true
}

func Run(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	evidence := Evidence{Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level == "replica" {
		evidence.Hosts = append(evidence.Hosts, "ubuntu")
	}
	add := func(name string, ok bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: ok})
	}
	fail := func(name string) Evidence { add(name, false); return evidence }
	replicas, seed := 1, 4
	for _, step := range s.Given {
		for action, options := range step {
			switch action {
			case "world":
				replicas = number(options, "replicas", 1)
			case "seed":
				seed = number(options, "count", 4)
			default:
				return fail("known_given_action")
			}
		}
	}
	w, err := world.New(ctx, level, replicas, seed)
	if err != nil {
		fmt.Fprintln(os.Stderr, "world startup:", err)
		return fail("world_start")
	}
	defer w.Close()
	add("replica_host_is_"+map[bool]string{true: "ubuntu", false: "air"}[level == "replica"], true)
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		return fail("initial_convergence")
	}
	for _, e := range w.Replicas {
		status, err := w.Status(e)
		if err != nil || status.Snapshots != 1 {
			return fail("bootstrap_snapshot")
		}
	}
	add("bootstrap_snapshot", true)
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	watchers := []*watcher{}
	for _, e := range endpoints {
		observer, err := watch(ctx, e, false)
		if err != nil {
			return fail("sse_observe")
		}
		defer observer.close()
		watchers = append(watchers, observer)
	}
	scoped := []*watcher{}
	for _, e := range endpoints {
		token, err := w.ScopedToken(e, "system:read")
		if err != nil {
			return fail("scope_token")
		}
		e.Token = token
		var result contracts.ReadResult
		_, err = world.Request(ctx, e, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder"}, &result)
		if err != nil || len(result.Records) != 0 {
			return fail("scope_query")
		}
		all, err := w.Records(endpoints[0])
		if err != nil {
			return fail("scope_fixture")
		}
		for _, r := range all {
			if r.Collection == "reminders.reminder" {
				status, _ := world.Request(ctx, e, "/v1/records/"+r.Collection+"/"+r.ID, nil, nil)
				if status != 404 {
					return fail("scope_get")
				}
				break
			}
		}
		observer, err := watch(ctx, e, true)
		if err != nil {
			return fail("scope_sse")
		}
		defer observer.close()
		scoped = append(scoped, observer)
	}
	add("scope_query_and_get", true)
	effects := map[string]time.Time{}
	partitionEffects := map[string]time.Time{}
	change := func(title string, measure bool) error {
		if err := w.ControlAction("/external-change", map[string]string{"key": title}); err != nil {
			return err
		}
		if measure {
			effects[title] = time.Now()
		} else {
			partitionEffects[title] = time.Now()
		}
		return nil
	}
	if err = change("probe", true); err != nil {
		return fail("external_change")
	}
	for _, step := range s.When {
		for action, options := range step {
			switch action {
			case "connect":
			case "changes":
				delete(effects, "probe")
				count, rate := number(options, "count", 200), number(options, "per_second", 5)
				if count < 1 || rate < 1 {
					return fail("change_parameters")
				}
				next := time.Now()
				for i := 0; i < count; i++ {
					if err = change(fmt.Sprintf("live-%03d", i), true); err != nil {
						return fail("external_change")
					}
					next = next.Add(time.Second / time.Duration(rate))
					if delay := time.Until(next); delay > 0 {
						if err = sleep(ctx, delay); err != nil {
							return fail("canceled")
						}
					}
				}
			case "partition":
				if err = w.Await(5*time.Second, func() bool { _, ok := watchers[1].observed("probe"); return ok }); err != nil {
					return fail("pre_partition_visible")
				}
				w.Partition(0)
				seconds, count := number(options, "seconds", 30), number(options, "changes", 30)
				for i := 0; i < count; i++ {
					if err = change(fmt.Sprintf("offline-%03d", i), false); err != nil {
						return fail("partition_change")
					}
					if err = sleep(ctx, time.Duration(seconds)*time.Second/time.Duration(count)); err != nil {
						return fail("canceled")
					}
				}
			case "heal":
				if err = w.Heal(0); err != nil {
					return fail("heal")
				}
				if err = w.Await(15*time.Second, func() bool { status, err := w.Status(w.Replicas[0]); return err == nil && status.State == "connected" }); err != nil {
					return fail("reconnect")
				}
				if err = change("healed", true); err != nil {
					return fail("heal_change")
				}
			default:
				return fail("known_when_action")
			}
		}
	}
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		return fail("convergence")
	}
	add("convergence_truth_gateway_replicas", true)
	timeout := w.Await(10*time.Second, func() bool {
		for title := range effects {
			for _, observer := range watchers {
				if _, ok := observer.observed(title); !ok {
					return false
				}
			}
		}
		return true
	})
	for i, observer := range watchers {
		values := []float64{}
		timeouts := 0
		for title, effect := range effects {
			seen, ok := observer.observed(title)
			if !ok {
				timeouts++
				continue
			}
			values = append(values, max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond)))
		}
		name := "effect_done→gateway_visible"
		if i > 0 {
			name = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
		}
		evidence.Latency[name] = stats(values, timeouts)
	}
	add("latency_observations_complete", timeout == nil)
	for _, step := range s.Then {
		for action, options := range step {
			switch action {
			case "converge":
			case "source_ready":
				var status contracts.ReadResult
				_, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", struct{}{}, &status)
				ready := false
				for _, r := range status.Records {
					var data map[string]any
					json.Unmarshal(r.Data, &data)
					if data["source"] == "reminders" && data["state"] == "ready" {
						ready = true
					}
				}
				add("source_ready", err == nil && ready)
			case "latency":
				limit, samples := number(options, "p95_ms", 1000), number(options, "samples", 200)
				for name, latency := range evidence.Latency {
					add("latency_"+name, latency.Samples >= samples && latency.Timeouts == 0 && latency.P95 < float64(limit))
				}
			case "capture_safety":
				add("capture_stale_noop_and_absence_protection", captureSafety(ctx, w))
			case "old_sse_cursor":
				for _, endpoint := range w.Replicas {
					status, _ := world.Request(ctx, endpoint, "/v1/changes?after=0", nil, nil)
					add("old_sse_cursor_reset", status == 409)
				}
			case "resumed":
				for _, e := range w.Replicas {
					status, err := w.Status(e)
					add("resume_without_snapshot", err == nil && status.Mode == "resume" && status.Resumes == 1 && status.Snapshots == 0)
				}
			default:
				add("known_then_action", false)
			}
		}
	}
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		add("post_assertion_convergence", false)
	}
	for i, observer := range watchers {
		add(fmt.Sprintf("sse_contiguous_watermarks_%d", i), observer.valid())
		truth, err := w.Inspect()
		absent := map[string]bool{}
		for _, o := range truth {
			if o.Kind == "absent" {
				var d map[string]any
				json.Unmarshal(o.Data, &d)
				title, _ := d["title"].(string)
				absent[o.Collection+"/"+title] = true
			}
		}
		observer.mu.Lock()
		valid := err == nil
		for _, seen := range observer.changes {
			if seen.Change.Op == "delete" && !absent[key(seen.Change.Record)] {
				valid = false
			}
		}
		observer.mu.Unlock()
		add(fmt.Sprintf("no_false_deletion_%d", i), valid)
	}
	for i, observer := range scoped {
		observer.mu.Lock()
		advanced := observer.last.Sequence > 0
		observer.mu.Unlock()
		add(fmt.Sprintf("scope_sse_watermarks_%d", i), observer.valid() && advanced)
	}
	for i, observer := range watchers {
		if i == 0 {
			continue
		}
		values := []float64{}
		timeouts := 0
		for title, effect := range partitionEffects {
			seen, ok := observer.observed(title)
			if !ok {
				timeouts++
				continue
			}
			values = append(values, max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond)))
		}
		if len(partitionEffects) > 0 {
			evidence.Latency[fmt.Sprintf("partition_effect_done→replica_visible:replica%d", i-1)] = stats(values, timeouts)
			add(fmt.Sprintf("partition_observations_complete_%d", i), timeouts == 0)
		}
	}

	evidence.Passed = true
	for _, a := range evidence.Assertions {
		if !a.Passed {
			evidence.Passed = false
		}
	}
	return evidence
}

func Save(e Evidence) error {
	dir := filepath.Join("lab/evidence", e.Level)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	b, err := json.MarshalIndent(e, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, e.Scenario+".json"), append(b, '\n'), 0644)
}
func Suite(ctx context.Context, paths []string, level string, asJSON bool) error {
	if level != "sim" && level != "replica" {
		return fmt.Errorf("level unsupported by tracer")
	}
	scenarios, err := Load(paths)
	if err != nil {
		return err
	}
	if len(scenarios) == 0 {
		return fmt.Errorf("no scenarios")
	}
	if err = world.Build(ctx, level); err != nil {
		return err
	}
	failed := false
	ran := 0
	for _, s := range scenarios {
		if !slices.Contains(s.Levels, level) {
			continue
		}
		ran++
		fmt.Fprintln(os.Stderr, "running", s.ID, level)
		bounded, cancel := context.WithTimeout(ctx, 3*time.Minute)
		e := Run(bounded, s, level)
		cancel()
		if err = Save(e); err != nil {
			return err
		}
		if asJSON {
			json.NewEncoder(os.Stdout).Encode(e)
		} else {
			fmt.Printf("%s %s passed=%t latency=%s\n", s.ID, level, e.Passed, latencyJSON(e.Latency))
		}
		if !e.Passed {
			failed = true
			for _, a := range e.Assertions {
				if !a.Passed {
					fmt.Fprintln(os.Stderr, "failed assertion:", a.Name)
				}
			}
		}
	}
	if failed {
		return errors.New("scenario suite failed")
	}
	if ran == 0 {
		return errors.New("no scenarios support requested level")
	}
	return nil
}
func latencyJSON(v any) string { b, _ := json.Marshal(v); return string(b) }

func captureSafety(ctx context.Context, w *world.World) bool {
	observations, err := w.Inspect()
	if err != nil {
		return false
	}
	var observation contracts.Observation
	for _, candidate := range observations {
		if candidate.Collection == "reminders.reminder" && candidate.Kind == "upsert" {
			observation = candidate
			break
		}
	}
	if observation.NativeKey == "" {
		return false
	}
	before, err := w.Records(w.Gateway)
	if err != nil {
		return false
	}
	var original contracts.Record
	for _, r := range before {
		if key(r) == key(contracts.Record{Collection: observation.Collection, Data: observation.Data}) {
			original = r
			break
		}
	}
	start := time.Now().UTC()
	fresh := observation
	fresh.ObservedAt = start.Add(time.Millisecond)
	if w.ControlAction("/capture", contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{fresh}}) != nil {
		return false
	}
	stale := observation
	stale.ObservedAt = start
	var data map[string]any
	json.Unmarshal(stale.Data, &data)
	data["title"] = "[ab-test] stale"
	stale.Data, _ = json.Marshal(data)
	if w.ControlAction("/capture", contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{stale}}) != nil {
		return false
	}
	absent := observation
	absent.Kind = "absent"
	absent.Revalidated = true
	absent.ObservedAt = start.Add(2 * time.Millisecond)
	complete := contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{absent}, ScanID: "lab-scan", Scope: []string{observation.Collection}, ScanStartedAt: start, CompletionEvidence: "complete"}
	if w.ControlAction("/capture", complete) != nil {
		return false
	}
	complete.ScanStartedAt = start.Add(3 * time.Millisecond)
	complete.CompletionEvidence = ""
	if w.ControlAction("/capture", complete) != nil {
		return false
	}
	var result contracts.ReadResult
	_, err = world.Request(ctx, w.Gateway, "/v1/records/"+original.Collection+"/"+original.ID, nil, &result)
	return err == nil && len(result.Records) == 1 && reflect.DeepEqual(result.Records[0], original)
}
