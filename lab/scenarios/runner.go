package scenarios

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
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
	Level   string                      `yaml:"level,omitempty"`
	Web     *WebOptions                 `yaml:"web,omitempty"`
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
	Status string `json:"status,omitempty"`
	Detail string `json:"detail,omitempty"`
}
type Latency struct {
	Unpaired int     `json:"unpaired,omitempty"`
	Samples  int     `json:"samples"`
	P50      float64 `json:"p50_ms"`
	P95      float64 `json:"p95_ms"`
	Max      float64 `json:"max_ms"`
	Timeouts int     `json:"timeouts"`
}
type Evidence struct {
	HostTelemetry  map[string]string  `json:"host_telemetry,omitempty"`
	HostConditions *HostConditions    `json:"host_conditions"`
	Status         string             `json:"status,omitempty"`
	RunID          string             `json:"run_id"`
	NetworkPath    string             `json:"network_path,omitempty"`
	Metrics        map[string]any     `json:"metrics,omitempty"`
	Scenario       string             `json:"scenario"`
	Level          string             `json:"level"`
	Passed         bool               `json:"passed"`
	GitSHA         string             `json:"git_sha"`
	At             time.Time          `json:"at"`
	Assertions     []Assertion        `json:"assertions"`
	Latency        map[string]Latency `json:"latency"`
	Hosts          []string           `json:"hosts"`
	NativeTimings  []NativeTiming     `json:"native_timings,omitempty"`
	Conditions     map[string]string  `json:"conditions,omitempty"`
}

type Seen struct {
	UptimeNS int64
	Change   contracts.Change
	At       time.Time
}
type watcher struct {
	snapshotResets  int
	mu              sync.Mutex
	seen            map[string]Seen
	changes         []Seen
	last            contracts.Cursor
	active          contracts.Cursor
	invalid         bool
	filtered        bool
	initialSequence uint64
	falseDelete     bool
	connected       chan struct{}
	cancel          context.CancelFunc
	done            chan struct{}
}

func key(r contracts.Record) string { return r.Collection + "/" + r.ID }
func recordTitle(r contracts.Record) string {
	var data map[string]any
	decodeData(r.Data, &data)
	title, _ := data["title"].(string)
	return title
}

func watch(ctx context.Context, e world.Endpoint, filtered bool, oracle func(contracts.Record) bool) (*watcher, error) {
	var result contracts.ReadResult
	if _, err := world.Request(ctx, e, "/v1/op/records.query", contracts.ReadQuery{Limit: 1}, &result); err != nil {
		return nil, err
	}
	ctx, cancel := context.WithCancel(ctx)
	w := &watcher{seen: map[string]Seen{}, last: result.Cursor, initialSequence: result.Cursor.Sequence, filtered: filtered, connected: make(chan struct{}), cancel: cancel, done: make(chan struct{})}
	go func() {
		defer close(w.done)
		ready := false
		for ctx.Err() == nil {
			w.mu.Lock()
			after := w.last
			w.mu.Unlock()
			req, err := http.NewRequestWithContext(ctx, "GET", fmt.Sprintf("%s/v1/changes?after=%s", e.URL, after.EventID()), nil)
			if err != nil {
				return
			}
			req.Host = e.Host
			req.Header.Set("Authorization", "Bearer "+e.Token)
			resp, err := http.DefaultClient.Do(req)
			if err == nil && resp.StatusCode == 200 {
				w.mu.Lock()
				w.active = after
				w.mu.Unlock()
				if !ready {
					close(w.connected)
					ready = true
				}
				scanner := bufio.NewScanner(resp.Body)
				scanner.Buffer(make([]byte, 4096), contracts.MaxFrame*2)
				reset := false
				for scanner.Scan() {
					line := scanner.Text()
					if line == "event: reset" {
						reset = true
						continue
					}
					if reset && strings.HasPrefix(line, "data: ") {
						w.resetSnapshot(ctx, e)
						break
					}
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
					if b.Partial || b.Epoch != w.last.Epoch || b.SubscriptionRevision != w.last.SubscriptionRevision || b.After != w.last.Sequence || b.Through < b.After {
						w.invalid = true
					}
					previous := b.After
					for _, c := range b.Changes {
						if c.Sequence <= previous || c.Sequence > b.Through {
							w.invalid = true
						}
						previous = c.Sequence
						if c.Op == "delete" && !oracle(c.Record) {
							w.falseDelete = true
						}
						if filtered && c.Record.Source != contracts.System {
							w.invalid = true
						}
						seen := Seen{Change: c, At: at, UptimeNS: world.UptimeNS()}
						w.seen[key(c.Record)] = seen
						w.changes = append(w.changes, seen)
					}
					w.last.Sequence = b.Through
					w.mu.Unlock()
				}
			}
			if resp != nil && resp.StatusCode == http.StatusConflict {
				w.resetSnapshot(ctx, e)
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
	for _, s := range w.seen {
		if s.Change.Record.Collection == "reminders.reminder" && recordTitle(s.Change.Record) == "[ab-test] "+title {
			return s, true
		}
	}
	return Seen{}, false
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
			return nil, fmt.Errorf("%s: %w", f, err)
		}
		if err = dec.Decode(&struct{}{}); err != io.EOF {
			return nil, fmt.Errorf("%s: expected one YAML document", f)
		}
		if s.Level != "" && len(s.Levels) == 0 {
			s.Levels = []string{s.Level}
		}
		if err = validateScenario(s); err != nil {
			return nil, fmt.Errorf("%s: %w", f, err)
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

func convergence(w *world.World) bool {
	truth, err := w.Inspect()
	if err != nil {
		return false
	}
	gateway, err := w.Records(w.Gateway)
	if err != nil {
		return false
	}
	principal, err := w.Principal(w.Gateway)
	if err != nil {
		return false
	}
	visibleObservations := []contracts.Observation{}
	for _, observation := range truth {
		if principal.Visibility()[contracts.CollectionSource(observation.Collection)] {
			visibleObservations = append(visibleObservations, observation)
		}
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return false
	}
	expected, err := truthRecords(visibleObservations, aliases)
	if err != nil || !reflect.DeepEqual(recordMultiset(withoutCommands(gateway), false), recordMultiset(expected, false)) {
		return false
	}
	gatewayCursor, err := w.Cursor(w.Gateway)
	if err != nil {
		return false
	}
	for _, endpoint := range w.Replicas {
		replicaCursor, err := w.Cursor(endpoint)
		if err != nil || (replicaCursor.Epoch != gatewayCursor.Epoch || replicaCursor.Sequence != gatewayCursor.Sequence) {
			return false
		}
		replica, err := w.Records(endpoint)
		principal, principalErr := w.Principal(endpoint)
		visibleGateway := []contracts.Record{}
		for _, record := range gateway {
			if principal.Visibility().Allows(record) {
				visibleGateway = append(visibleGateway, record)
			}
		}
		if err != nil || principalErr != nil || !reflect.DeepEqual(recordMultiset(withoutCommands(replica), true), recordMultiset(withoutCommands(visibleGateway), true)) {
			return false
		}
	}
	return true
}

func runScenario(ctx context.Context, s Scenario, level string) Evidence {
	if s.ID == "HELPER-RELEASE-COMMANDS" {
		return runHelperReleaseCommands(ctx, s, level)
	}
	if s.ID == "CORE-SIM-PROGRESS-RECOVERY" {
		return runSimProgressRecovery(ctx, s, level)
	}
	if s.ID == "CORE-CAPTURE-ALIASES" {
		return runCaptureAliases(ctx, s, level)
	}
	if s.ID == "CORE-READMODEL-UPDATE-PERF" {
		return runReadmodelUpdatePerf(ctx, s, level)
	}
	if s.ID == "CORE-SEARCH-UPDATE-PERF" {
		return runSearchUpdatePerf(ctx, s, level)
	}
	if s.ID == "WEB-FIXTURE-SNAPSHOT" {
		return runWebFixtureSnapshot(ctx, s, level)
	}
	if s.ID == "WEB-RM-REMINDERS-HARDENING" {
		return runRemindersHardening(ctx, s, level)
	}
	if s.ID == "WEB-RM-REMINDERS-ITEMS" {
		return runRemindersItems(ctx, s, level)
	}
	if s.ID == "WEB-RM-REMINDERS-LISTS" {
		return runRemindersSidebar(ctx, s, level)
	}
	if s.ID == "HELPER-IPC-RESPONSIVE" {
		return runHelperIPC(ctx, s, level)
	}
	if s.ID == "NATIVE-MEDIA-YIELD" {
		return runMediaYield(ctx, s, level)
	}
	if s.ID == "NATIVE-MEDIA-POSTERS" {
		return runNativeMediaPosters(ctx, s, level)
	}
	if s.ID == "NATIVE-MEDIA-EXPORT-SAFETY" {
		return runMediaExportSafety(ctx, s, level)
	}
	if s.ID == "CORE-REPLICA-CAPTURE-OBSERVERS" {
		return runAgentAccess(ctx, s, level, runCaptureObservers)
	}
	if s.ID == "HELPER-SIGNED-APP-CACHE" {
		return runNativeAppCache(ctx, s, level)
	}
	if s.ID == "REM-STORE-PERF" {
		return runRemindersStorePerf(ctx, s, level)
	}
	if s.ID == "CORE-REPLICA-CAPTURE-STAGING" || s.ID == "CORE-REPLICA-CAPTURE-LEGACY" {
		return runReplicaCaptureStaging(ctx, s, level)
	}
	if s.ID == "CORE-REPLICA-CAPTURE-ATOMIC" {
		return runReplicaCaptureAtomic(ctx, s, level)
	}
	if s.ID == "REM-STORE-READ" || s.ID == "REM-STORE-DECODE" {
		return runRemindersStore(ctx, s, level)
	}
	if s.ID == "WEB-RM-REMINDERS-DEMO" {
		return runRemindersDemo(ctx, s, level)
	}
	if level == "web" {
		return runWeb(ctx, s)
	}
	if s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-MEDIA-CAP" {
		return runMediaVariants(ctx, s, level)
	}
	if s.Feature == "web.demo" {
		return runDemoDataset(ctx, s, level)
	}
	if s.Feature == "web.relay" {
		if s.ID == "WEB-RELAY-REPLICA" || s.ID == "WEB-RELAY-NATIVE-AGENTS" || s.ID == "WEB-RELAY-REPLICA-BLOBS" {
			return runRelayReplica(ctx, s, level)
		}
		if s.ID == "WEB-RELAY-AGENTS" {
			return runRelayAgents(ctx, s, level)
		}
		if s.ID == "WEB-RELAY-CONNECT" || s.ID == "WEB-RELAY-COMMAND" || s.ID == "WEB-RELAY-PUBLIC-AGENTS" {
			return runRelayConnect(ctx, s, level)
		}
		if s.ID == "WEB-RELAY-PAIRING" {
			return runRelayPairing(ctx, s, level)
		}
		return runRelayAuth(ctx, s, level)
	}
	if s.Feature == "web.remote" {
		return runRemote(ctx, s, level)
	}
	if s.Feature == "web.messages.read" && strings.HasPrefix(s.ID, "MSG-") {
		return runRichMessages(ctx, s, level)
	}
	if s.ID == "WEB-RM-MEDIA-COLLECTION" {
		return runMediaCollection(ctx, s, level)
	}
	if s.Feature == "web.readmodels" {
		return runReadModels(ctx, s, level)
	}
	if s.ID == "CORE-ITERATOR-POOL" || s.ID == "CORE-SNAPSHOT-POOL" {
		return runWebServer(ctx, s, level)
	}
	if s.ID == "MSG-DELETE-VISIBILITY" {
		return runNativeDeleteVisibility(ctx, s, level)
	}
	if s.ID == "CON-RECIPIENT-LOOKUP" {
		return runRecipientLookup(ctx, s, level)
	}
	if s.ID == "WEB-SOURCES-STATUS" || s.ID == "WEB-COMMAND-RESOLVE" {
		return runWebServer(ctx, s, level)
	}
	if s.ID == "MSG-SELFTEST" || s.ID == "MSG-RESEND" || s.ID == "MSG-READSTATE-REAL" || s.ID == "MSG-DELETE-REAL" || s.ID == "MSG-COMMAND-OBSERVED-IDS" || s.ID == "MSG-FAILED-OBSERVED-IDS" || s.ID == "MSG-COMMAND-PARTS" {
		return runMessagesCommands(ctx, s, level)
	}
	if s.ID == "MSG-MEDIA-CLEANUP" {
		return runMediaCleanupProof(ctx, s, level)
	}
	if s.ID == "MSG-INTERACTIVE-GATE" {
		if level == "real" {
			return runMessagesCommands(ctx, s, level)
		}
		return runWebServer(ctx, s, level)
	}
	if s.ID == "WEB-UPLOAD-CONTROL-PRODUCTION" {
		return runUploadControlProduction(ctx, s, level)
	}
	if s.ID == "WEB-UPLOAD-REPLICA-CHECKPOINT" && !uploadControlLabEnabled() {
		return runUploadTagged(ctx, s, level, true)
	}
	if strings.HasPrefix(s.ID, "WEB-UPLOAD-REPLICA") {
		return runUploadReplica(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "WEB-UPLOAD-") {
		return runWebUpload(ctx, s, level)
	}
	if s.ID == "WEB-AGENT-OAUTH" {
		return runAgentAccess(ctx, s, level, runAgentOAuth)
	}
	if s.ID == "WEB-AGENT-LEVELS" {
		return runAgentAccess(ctx, s, level, runAgentLevels)
	}
	if s.Feature == "web.frontdoor" || s.Feature == "web.agents" {
		return runWebServer(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "USE-") {
		return runUsage(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "CAL-") {
		return runCalendarScenario(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "NOTE-") {
		return runNotes(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "CON-") {
		return runContacts(ctx, s, level)
	}
	if strings.HasPrefix(s.Feature, "messages.") && s.Feature != "messages.read" {
		return runMessagesCommands(ctx, s, level)
	}
	if s.ID == "CORE-BLOB-NATIVE-STAGE" || s.ID == "CORE-BLOB-STAGE-DURING-TRANSFER" || s.ID == "CORE-BLOB-STAGE-SAME-DIGEST" {
		return runNativeBlobStage(ctx, s, level)
	}
	if nativeScenario(s.ID, level) {
		return runNativeScenario(ctx, s, level)
	}
	if s.Feature == "messages.read" && strings.HasPrefix(s.ID, "MSG-") {
		return runMessages(ctx, s, level)
	}
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	evidence := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level == "replica" {
		evidence.Hosts = append(evidence.Hosts, "ubuntu")
	}
	add := func(name string, ok bool) {
		evidence.Assertions = append(evidence.Assertions, Assertion{Name: name, Passed: ok})
	}
	fail := func(name string) Evidence { add(name, false); return evidence }
	if len(s.Then) == 1 {
		if options, ok := s.Then[0]["load_rejected"]; ok {
			path := options["path"].(string)
			_, err := Load([]string{path})
			add("malformed_scenario_rejected", err != nil && strings.Contains(err.Error(), "p95_ms"))
			evidence.Passed = evidence.Assertions[0].Passed
			if err != nil {
				evidence.Assertions[0].Detail = err.Error()
			}
			return evidence
		}
	}
	replicas, seed := 1, 4
	for _, step := range s.Given {
		for action, options := range step {
			switch action {
			case "world":
				replicas = number(options, "replicas", 1)
				if options["gateway_dials"] == true {
					ctx = world.WithGatewayDials(ctx)
				}
			case "seed":
				seed = number(options, "count", 4)
			default:
				return fail("known_given_action")
			}
		}
	}
	for _, step := range s.When {
		if options, ok := step["robustness"]; ok {
			return runRobust(ctx, s, level, options, evidence)
		}
	}
	options := []world.BlobOptions{}
	if strings.HasPrefix(s.ID, "CORE-BLOB-") {
		opt := world.BlobOptions{GraceMS: 500}
		if s.ID == "CORE-BLOB-CAP" {
			opt.Cap = 1 << 20
		}
		opt.Corrupt = s.ID == "CORE-BLOB-CORRUPT"
		opt.Slow = s.ID == "CORE-BLOB-INTERRUPT"
		options = append(options, opt)
	}
	w, err := world.New(ctx, level, replicas, seed, options...)
	if err != nil {
		fmt.Fprintln(os.Stderr, "world startup:", err)
		return fail("world_start")
	}
	defer w.Close()
	evidence.NetworkPath = w.NetworkPath
	add("feed_network_"+w.NetworkPath, true)
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
		observer, err := watch(ctx, e, false, func(r contracts.Record) bool { return truthAbsent(w, r) })
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
		_, err = world.Request(ctx, e, "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.reminder", IncludeAliases: true}, &result)
		if err != nil || len(result.Records) != 0 || len(result.Aliases) != 0 {
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
		observer, err := watch(ctx, e, true, func(r contracts.Record) bool { return truthAbsent(w, r) })
		if err != nil {
			return fail("scope_sse")
		}
		defer observer.close()
		scoped = append(scoped, observer)
	}
	add("scope_query_and_get", true)
	for _, step := range s.When {
		if _, ok := step["command_journey"]; ok {
			if w.ControlAction("/external-change", map[string]string{"key": "command-probe"}) != nil {
				return fail("command_probe")
			}
			effectStart := time.Now()
			if w.Await(5*time.Second, func() bool {
				for _, observer := range watchers {
					if _, ok := observer.observed("command-probe"); !ok {
						return false
					}
				}
				return true
			}) != nil {
				return fail("healthy_command_probe_visible")
			}
			probeSeen := make([]Seen, len(watchers))
			for i, observer := range watchers {
				probeSeen[i], _ = observer.observed("command-probe")
			}
			runCommands(ctx, w, s.ID, &evidence, watchers)
			if s.ID == "CORE-CMD-ERA" {
				head, _ := w.Cursor(w.Gateway)
				all := append(append([]*watcher{}, watchers...), scoped...)
				reset := w.Await(5*time.Second, func() bool {
					for _, observer := range all {
						observer.mu.Lock()
						epoch := observer.active.Epoch
						observer.mu.Unlock()
						if epoch != head.Epoch {
							return false
						}
					}
					return true
				})
				add("all_sse_observers_resume_new_epoch", reset == nil)
				add("post_restore_probe", w.ControlAction("/external-change", map[string]string{"key": "post-restore-probe"}) == nil)
				add("post_restore_probe_committed", w.ControlAction("/flush", struct{}{}) == nil)
				head, _ = w.Cursor(w.Gateway)
				advanced := w.Await(5*time.Second, func() bool {
					for _, observer := range all {
						observer.mu.Lock()
						last := observer.last
						observer.mu.Unlock()
						if last.Epoch != head.Epoch || last.Sequence < head.Sequence {
							return false
						}
					}
					return true
				})
				add("all_sse_observers_advance_after_restore", advanced == nil)
			}
			for i, observer := range watchers {
				seen := probeSeen[i]
				ok := !seen.At.IsZero()
				values := []float64{}
				timeouts := 0
				if ok {
					values = append(values, max(0, float64(seen.At.Sub(effectStart))/float64(time.Millisecond)))
				} else {
					timeouts = 1
				}
				name := "effect_done→gateway_visible"
				if i > 0 {
					name = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
				}
				evidence.Latency[name] = stats(values, timeouts)
				add(fmt.Sprintf("metadata_latency_below_1s_%d", i), ok && seen.At.Sub(effectStart) < time.Second)
				add(fmt.Sprintf("sse_contiguous_watermarks_%d", i), observer.valid())
				observer.mu.Lock()
				noFalseDelete := !observer.falseDelete
				observer.mu.Unlock()
				add(fmt.Sprintf("no_false_deletion_%d", i), noFalseDelete)
			}
			for i, observer := range scoped {
				observer.mu.Lock()
				advanced := observer.last.Sequence > observer.initialSequence
				observer.mu.Unlock()
				add(fmt.Sprintf("scope_sse_watermarks_%d", i), observer.valid() && advanced)
			}
			recordIntegrity(&evidence, w, nil)
			evidence.Passed = len(evidence.Assertions) > 0
			for _, a := range evidence.Assertions {
				evidence.Passed = evidence.Passed && (a.Passed || a.Status == "unverified")
			}
			return evidence
		}
	}
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
			case "truth_multiplicity":
				data := json.RawMessage(`{"title":"[ab-test] duplicate","completed":false}`)
				for _, key := range []string{"duplicate-a", "duplicate-b"} {
					if err = w.ControlAction("/external-change", map[string]any{"key": key, "data": data, "omit_capture": key == "duplicate-b"}); err != nil {
						return fail("multiplicity_fixture")
					}
				}
				if err = w.ControlAction("/flush", struct{}{}); err != nil {
					return fail("multiplicity_flush")
				}
				truth, truthErr := w.Inspect()
				records, recordsErr := w.Records(w.Gateway)
				truthCount, gatewayCount := 0, 0
				for _, o := range truth {
					if recordTitle(contracts.Record{Data: o.Data}) == "[ab-test] duplicate" {
						truthCount++
					}
				}
				for _, record := range records {
					if recordTitle(record) == "[ab-test] duplicate" {
						gatewayCount++
					}
				}
				add("multiplicity_fault_two_truth_one_gateway", truthErr == nil && recordsErr == nil && truthCount == 2 && gatewayCount == 1)
				detected := truthErr == nil && recordsErr == nil && truthCount == 2 && gatewayCount == 1 && !convergence(w)
				add("truth_multiplicity_mismatch_detected", detected)
				evidence.Passed = detected
				return evidence
			case "blobs":
				if !blobJourney(ctx, w, s.ID, number(options, "rows", 10000), &evidence, add) {
					return evidence
				}
				if s.ID == "CORE-BLOB-REFERENCE-INDEX" {
					evidence.Passed = true
					for _, assertion := range evidence.Assertions {
						evidence.Passed = evidence.Passed && assertion.Passed
					}
					return evidence
				}
			case "gateway_commands":
				gatewayCommands(ctx, w, &evidence)
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
			case "disconnect":
				if w.Await(5*time.Second, func() bool { return convergence(w) }) != nil {
					return fail("pre_disconnect_convergence")
				}
				before, err := w.Records(w.Replicas[0])
				if err != nil {
					return fail("reads_before_disconnect")
				}
				w.Blackhole()
				if err = w.Await(15*time.Second, func() bool {
					status, err := w.Status(w.Replicas[0])
					return err == nil && status.State == "disconnected"
				}); err != nil {
					return fail("disconnected_within_15_seconds")
				}
				after, err := w.Records(w.Replicas[0])
				if err != nil || !reflect.DeepEqual(before, after) {
					return fail("local_reads_survive_disconnect")
				}
				add("silent_blackhole_detected_within_15_seconds_reads_continue", true)
				if err = w.Heal(0); err != nil {
					return fail("heal")
				}
				if err = change("reconnected", true); err != nil {
					return fail("reconnect_change")
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
	if s.ID == "CORE-GATEWAY-DIALS" {
		delete(effects, "healed")
	}
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		fmt.Fprintln(os.Stderr, "convergence diagnosis:", convergenceFailure(w))
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
	if s.ID == "CORE-LIVE-LATENCY" {
		retainLiveSamples(&evidence, effects, watchers)
	}
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
			case "gateway_revoke":
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
			case "sse_resume":
				for i, endpoint := range endpoints {
					add(fmt.Sprintf("sse_full_cursor_resume_%d", i), sseResume(ctx, w, endpoint))
				}
			case "sse_idle":
				for i, endpoint := range endpoints {
					add(fmt.Sprintf("sse_idle_heartbeats_%d", i), sseIdle(ctx, w, endpoint))
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
					add("resume_without_snapshot", err == nil && status.Mode == "resume" && status.Resumes >= 1 && status.Snapshots == 1)
				}
			default:
				add("known_then_action", false)
			}
		}
	}
	if err = w.Await(15*time.Second, func() bool { return convergence(w) }); err != nil {
		add("post_assertion_convergence", false)
	}
	recordIntegrity(&evidence, w, nil)
	for i, observer := range watchers {
		add(fmt.Sprintf("sse_contiguous_watermarks_%d", i), observer.valid())
		truth, err := w.Inspect()
		aliases, aliasErr := gatewayAliases(w)
		absent := map[string]bool{}
		for _, o := range truth {
			if o.Kind == "absent" {
				absent[o.Collection+"/"+aliases[aliasKey(contracts.CollectionSource(o.Collection), o.Collection, o.NativeKey)]] = true
			}
		}

		observer.mu.Lock()
		valid := err == nil && aliasErr == nil && !observer.falseDelete
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
		advanced := observer.last.Sequence > observer.initialSequence
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

	if s.ID == "CORE-GATEWAY-DIALS" {
		gatewayRevoke(ctx, w, &evidence)
	}
	evidence.Passed = true
	for _, a := range evidence.Assertions {
		if !a.Passed && a.Status != "unverified" {
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
	if level != "sim" && level != "replica" && level != "real" && level != "web" {
		return fmt.Errorf("level unsupported by tracer")
	}
	scenarios, err := Load(paths)
	if err != nil {
		return err
	}
	if len(scenarios) == 0 {
		return fmt.Errorf("no scenarios")
	}
	packagingOnly := true
	for _, scenario := range scenarios {
		packagingOnly = packagingOnly && scenario.Feature == "packaging.app"
	}
	if !packagingOnly {
		if err = world.Build(ctx, level); err != nil {
			return err
		}
	}
	for _, scenario := range scenarios {
		if scenario.ID == "CORE-UPGRADE" || scenario.ID == "CORE-RESTORE-OLD-SCHEMA" || strings.Contains(scenario.ID, "REVIEW-UPGRADE") {
			if err = world.BuildLegacy(ctx); err != nil {
				return err
			}
			break
		}
	}
	failed := false
	ran := 0
	for _, s := range scenarios {
		if !slices.Contains(s.Levels, level) {
			continue
		}
		ran++
		fmt.Fprintln(os.Stderr, "running", s.ID, level)
		duration := 3 * time.Minute
		if s.ID == "CORE-SEARCH-UPDATE-PERF" {
			duration = 10 * time.Minute
		}
		if level == "real" && (s.ID == "CORE-LIVE-LATENCY" || s.ID == "REM-CRUD") {
			duration = 8 * time.Minute
		}
		if s.Feature == "web.relay" && level == "replica" {
			duration = 5 * time.Minute
		}
		if s.ID == "WEB-RELAY-REPLICA" {
			duration = 10 * time.Minute
		}
		if s.Feature == "web.readmodels" || s.Feature == "web.messages.read" {
			duration = 15 * time.Minute
		}
		if level == "real" && (s.Feature == "messages.actions" || s.Feature == "messages.groups" || s.Feature == "web.media") {
			duration = 8 * time.Minute
		}
		if s.ID == "CAL-CRUD" && level != "sim" {
			duration = 6 * time.Minute
		}
		if s.ID == "CORE-BENCH" {
			duration = 10 * time.Minute
			if os.Getenv("APPLE_BRIDGE_LAB_CONTENDED_BENCH") == "1" {
				duration = 35 * time.Minute
			}
		}
		if s.Feature == "messages.read" && level != "sim" {
			duration = 20 * time.Minute
		}
		if s.ID == "HELPER-SIGNED-APP-CACHE" || s.ID == "HELPER-RELEASE-COMMANDS" {
			duration = 10 * time.Minute
		}
		if s.ID == "HELPER-SESSION-SOAK" {
			duration = 12 * time.Minute
		}
		if level == "web" && s.Web != nil {
			duration = time.Duration(max(1, len(s.Web.Browsers)*len(s.Web.Themes)*len(s.Web.Layouts))) * webRunBudget(s)
		}
		bounded, cancel := context.WithTimeout(ctx, duration)
		secretBefore, secretWasSet := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
		e := Run(bounded, s, level)
		cancel()
		secretAfter, secretIsSet := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
		restored := secretBefore == secretAfter && secretWasSet == secretIsSet
		e.Assertions = append(e.Assertions, Assertion{Name: "credential_backend_override_restored", Passed: restored})
		e.Passed = e.Passed && restored
		if err = saveIncomingEchoEvidence(e); err != nil {
			return err
		}
		if err = Save(e); err != nil {
			return err
		}
		if asJSON {
			json.NewEncoder(os.Stdout).Encode(e)
		} else {
			fmt.Printf("%s %s passed=%t quiet=%t latency=%s\n", s.ID, level, e.Passed, e.HostConditions.Quiet, latencyJSON(e.Latency))
		}
		if !e.Passed {
			failed = true
			for _, a := range e.Assertions {
				if !a.Passed && a.Status != "unverified" {
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
	aliases, err := gatewayAliases(w)
	if err != nil {
		return false
	}
	id := aliases[aliasKey(contracts.CollectionSource(observation.Collection), observation.Collection, observation.NativeKey)]
	for _, r := range before {
		if r.Collection == observation.Collection && r.ID == id {
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
	if err != nil || len(result.Records) != 1 || !reflect.DeepEqual(result.Records[0], original) {
		return false
	}
	if w.ControlAction("/external-change", map[string]string{"key": "tombstone-safety"}) != nil || w.ControlAction("/flush", struct{}{}) != nil {
		return false
	}
	truth, err := w.Inspect()
	if err != nil {
		return false
	}
	var tombstone contracts.Observation
	for _, o := range truth {
		if o.NativeKey == "tombstone-safety" {
			tombstone = o
		}
	}
	if w.ControlAction("/external-change", map[string]any{"key": "tombstone-safety", "delete": true}) != nil || w.ControlAction("/flush", struct{}{}) != nil {
		return false
	}
	repeated := tombstone
	repeated.Kind = "absent"
	repeated.Revalidated = true
	repeated.ObservedAt = time.Now().UTC().Add(time.Millisecond)
	capture := contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{repeated}, ScanID: "repeated-absence", Scope: []string{tombstone.Collection}, ScanStartedAt: repeated.ObservedAt, CompletionEvidence: "complete"}
	if w.ControlAction("/capture", capture) != nil {
		return false
	}
	tombstone.ObservedAt = repeated.ObservedAt.Add(-time.Microsecond)
	if w.ControlAction("/capture", contracts.Capture{Source: contracts.Reminders, Observations: []contracts.Observation{tombstone}}) != nil {
		return false
	}
	return convergence(w) || w.Await(5*time.Second, func() bool { return convergence(w) }) == nil
}

func truthAbsent(w *world.World, r contracts.Record) bool {
	observations, err := w.Inspect()
	if err != nil {
		return false
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return false
	}
	for _, o := range observations {
		if o.Collection == r.Collection && aliases[aliasKey(contracts.CollectionSource(o.Collection), o.Collection, o.NativeKey)] == r.ID {
			return o.Kind == "absent"
		}
	}
	return false
}

func decodeData(raw json.RawMessage, value any) error {
	decoder := json.NewDecoder(strings.NewReader(string(raw)))
	decoder.UseNumber()
	return decoder.Decode(value)
}

func withoutCommands(records []contracts.Record) []contracts.Record {
	out := []contracts.Record{}
	for _, r := range records {
		if r.Collection != "system.command" {
			out = append(out, r)
		}
	}
	return out
}

func retainLiveSamples(evidence *Evidence, effects map[string]time.Time, watchers []*watcher) {
	titles := make([]string, 0, len(effects))
	for title := range effects {
		titles = append(titles, title)
	}
	slices.SortFunc(titles, func(a, b string) int { return effects[a].Compare(effects[b]) })
	samples := make([]map[string]any, 0, len(titles))
	for index, title := range titles {
		effect := effects[title]
		row := map[string]any{"index": index, "effect_at": effect.UTC().Format(time.RFC3339Nano)}
		for endpoint, observer := range watchers {
			seen, ok := observer.observed(title)
			if !ok {
				continue
			}
			name := "gateway"
			if endpoint > 0 {
				name = fmt.Sprintf("replica%d", endpoint-1)
			}
			row[name+"_visible_at"] = seen.At.UTC().Format(time.RFC3339Nano)
			row[name+"_ms"] = max(0, float64(seen.At.Sub(effect))/float64(time.Millisecond))
		}
		samples = append(samples, row)
	}
	if evidence.Metrics == nil {
		evidence.Metrics = map[string]any{}
	}
	evidence.Metrics["live_samples"] = samples
}
