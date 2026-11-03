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
	Implementations world.Implementations `json:"implementations"`
	Impl            string                `json:"impl"`
	HostTelemetry   map[string]string     `json:"host_telemetry,omitempty"`
	HostConditions  *HostConditions       `json:"host_conditions"`
	Status          string                `json:"status,omitempty"`
	RunID           string                `json:"run_id"`
	NetworkPath     string                `json:"network_path,omitempty"`
	Metrics         map[string]any        `json:"metrics,omitempty"`
	Scenario        string                `json:"scenario"`
	Level           string                `json:"level"`
	Passed          bool                  `json:"passed"`
	GitSHA          string                `json:"git_sha"`
	At              time.Time             `json:"at"`
	Assertions      []Assertion           `json:"assertions"`
	Latency         map[string]Latency    `json:"latency"`
	Hosts           []string              `json:"hosts"`
	NativeTimings   []NativeTiming        `json:"native_timings,omitempty"`
	Conditions      map[string]string     `json:"conditions,omitempty"`
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
	return loadLevel(paths, "")
}

func loadLevel(paths []string, level string) ([]Scenario, error) {
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
		if level == "real" && (s.ID == "MAC-REPLICA" || s.ID == "FILE-REPLICA") && slices.Contains(s.Levels, "replica") {
			s.Levels = append(s.Levels, "real")
		}
		if level != "" && !slices.Contains(s.Levels, level) {
			continue
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
	if s.ID == "APP-SHELL-SIM" && level == "sim" {
		return runIPhoneShell(ctx, s, level)
	}
	if s.ID == "QUERY-AGENT" {
		return runQueryAgent(ctx, s, level)
	}
	if level == "real" && isIPhoneSensorReal(s.ID) {
		return runIPhoneSensorReal(ctx, s)
	}
	if isIPhoneRealScenario(s.ID, level) {
		return runIPhoneReal(ctx, s, level)
	}
	if level == "real" && s.ID == "APP-BUILD-REAL" {
		return Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Status: "limited", Assertions: []Assertion{{Name: "real_device_acceptance", Detail: "Real iPhone acceptance harness is not implemented; generic replica proof cannot qualify the phone"}}}
	}
	if s.ID == "APP-DEVICE-SIM" || s.ID == "APP-DEVICE-REAL" {
		return runIPhoneCapability(ctx, s, level, "ios/Capabilities/DeviceContext/DeviceModel.swift", "lab/ios/DeviceProof.swift")
	}
	if s.ID == "APP-MOTION-SIM" || s.ID == "APP-MOTION-REAL" {
		return runIPhoneCapability(ctx, s, level, "ios/Capabilities/Motion/MotionModel.swift", "lab/ios/MotionProof.swift")
	}
	if s.Feature == "iphone.alarms" {
		return runIPhoneAlarm(ctx, s, level)
	}
	if s.Feature == "iphone.screentime" {
		return runIPhoneScreenTime(ctx, s, level)
	}
	if s.ID == "APP-LOC-SIM" || s.ID == "APP-LOC-REAL" {
		return runIPhoneLocation(ctx, s, level)
	}
	if s.ID == "APP-LIVE-SIM" || s.ID == "APP-LIVE-REAL" {
		return runIPhoneLive(ctx, s, level)
	}

	if s.Feature == "iphone.capture" {
		return runIPhoneCapture(ctx, s, level)
	}
	if s.ID == "APP-APPROVE-SIM" {
		return runIPhoneCommand(ctx, s, level)
	}
	if s.ID == "APP-NOTIFY-SIM" {
		return runIPhoneCommand(ctx, s, level)
	}
	if s.ID == "APP-CMD-SIM" {
		return runIPhoneCommand(ctx, s, level)
	}
	if s.ID == "APP-PAIR-SIM" {
		return runIPhonePair(ctx, s, level)
	}
	if s.ID == "APP-HEALTH-WRITE-SIM" || s.ID == "APP-HEALTH-WRITE-REAL" {
		return runIPhoneHealthWrite(ctx, s, level)
	}
	if s.ID == "APP-HEALTH-SIM" || s.ID == "APP-HEALTH-REAL" {
		return runIPhoneHealth(ctx, s, level)
	}
	if s.ID == "WEB-MSG-REAL-READ" {
		return runWebMessagesRead(ctx, s, level)
	}
	if s.Feature == "maps.queries" {
		return runMapsQueries(ctx, s, level)
	}
	if s.Feature == "mail.commands" || s.Feature == "mail.capture" || s.Feature == "mail.body" {
		return runMail(ctx, s, level)
	}
	if s.Feature == "audio.queries" {
		return runAudioQueries(ctx, s, level)
	}
	if s.Feature == "capture.commands" {
		return runCapture(ctx, s, level)
	}
	if s.Feature == "vision.queries" {
		return runVisionQueries(ctx, s, level)
	}
	if s.Feature == "packaging.setup" {
		return runSetup(ctx, s, level)
	}
	if s.Feature == "core.permissions" {
		return runPermissionStatus(ctx, s, level)
	}
	if s.Feature == "helper.siri_intents" {
		return runSiriIntents(ctx, s, level)
	}
	if s.Feature == "focus.pack" {
		return runFocusPack(ctx, s, level)
	}
	if s.Feature == "clock.pack" {
		return runClockPack(ctx, s, level)
	}
	if s.Feature == "files.capture" || s.Feature == "files.commands" || s.Feature == "files.transform" {
		return runFiles(ctx, s, level)
	}
	if s.Feature == "shortcuts.actions" {
		return runActions(ctx, s, level)
	}
	if s.Feature == "shortcuts.runner" {
		return runShortcutsRunner(ctx, s, level)
	}
	if s.Feature == "core.queries" {
		return runQueries(ctx, s, level)
	}
	if s.ID == "CORE-SHORTCUT-SIGN" {
		return runShortcutSign(ctx, s, level)
	}
	if s.ID == "RMT-PRIVATE-CONNECT" {
		return runPrivateConnect(ctx, s, level)
	}
	if s.ID == "RELAY-CT-INDEPENDENT" || s.ID == "RELAY-CAA-ACCOUNT" {
		return runRelayCertificates(ctx, s, level)
	}
	if s.ID == "RELAY-ROTATE-GATED" || s.ID == "RELAY-TLS-SPLICE" {
		return runRelayContainment(ctx, s, level)
	}
	if s.Feature == "remote.relay" {
		return runRelayService(ctx, s, level)
	}
	if s.ID == "CON-GROUPS" {
		return runContactGroups(ctx, s, level)
	}
	if s.ID == "WEB-CON-LIST" {
		return runContactsList(ctx, s, level)
	}
	if s.ID == "HELPER-RELEASE-COMMANDS" {
		return runHelperReleaseCommands(ctx, s, level)
	}
	if s.ID == "SAF-CAPTURE" || s.ID == "SAF-IDLE" || s.ID == "SAF-HISTORY-REAL" || s.ID == "SAF-BOOKMARKS-REAL" {
		return runSafariCapture(ctx, s, level)
	}

	if s.ID == "CORE-SOURCE-SUPPORT" {
		return runSourceSupport(ctx, s, level)
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
	if s.ID == "SESSION-PROXY-BYTES" {
		return runSessionProxy(ctx, s, level)
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
	if s.ID == "WEB-REM-PARITY" && level != "web" {
		return runRemindersSections(ctx, level, Evidence{RunID: contracts.NewID("reminders-parity"), Scenario: s.ID, Level: level, At: time.Now().UTC()})
	}
	if level == "web" {
		return runWeb(ctx, s)
	}
	if s.ID == "WEB-CAL-WRITE" && (level == "sim" || level == "real") {
		return runCalendarScenario(ctx, s, level)
	}
	if s.ID == "WEB-REM-WRITE" && level == "real" {
		return runRemindersWriteReal(ctx, s, level)
	}
	if s.ID == "WEB-MEDIA-VARIANTS" || s.ID == "WEB-MEDIA-CAP" {
		return runMediaVariants(ctx, s, level)
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
	if s.ID == "SEC-REMOTE-ACCESS" {
		return runRemoteSecurity(ctx, s, level)
	}
	if s.ID == "CONN-CLAUDE-CODE" && level == "real" {
		return runRelayService(ctx, s, level)
	}
	if (s.ID == "CONN-CHATGPT" || s.ID == "CONN-CLAUDE-AI") && level == "sim" {
		return runConnectorApproval(ctx, s, level)
	}
	if (s.ID == "CONN-CHATGPT" || s.ID == "CONN-CLAUDE-AI") && level == "real" {
		return runConnectorReal(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "RMT-") || strings.HasPrefix(s.ID, "CONN-") {
		return runRemoteFlow(ctx, s, level)
	}
	if s.Feature == "web.remote" {
		return runRemote(ctx, s, level)
	}
	if s.ID == "MSG-DECODE-BALLOON" || s.ID == "MSG-DECODE-FORMATTING" || s.ID == "MSG-READ-RICH-FIELDS" {
		return runRichMessages(ctx, s, level)
	}
	if s.ID == "WEB-RM-MEDIA-COLLECTION" {
		return runMediaCollection(ctx, s, level)
	}
	if s.ID == "WEB-RM-THREAD-ORIGINATOR" || s.ID == "WEB-RM-THREAD-ORIGINATOR-PERF" {
		return runThreadOriginator(ctx, s, level)
	}
	if s.ID == "WEB-RM-SEARCH" {
		return runMessageSearch(ctx, s, level)
	}
	if s.Feature == "web.readmodels" {
		return runReadModels(ctx, s, level)
	}
	if s.ID == "CORE-SNAPSHOT-POOL" {
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
	if s.ID == "MSG-OWNER-TOKEN-SELFCHAT" || s.ID == "MSG-SELFTEST" || s.ID == "MSG-RESEND" || s.ID == "MSG-READSTATE-REAL" || s.ID == "MSG-DELETE-REAL" || s.ID == "MSG-COMMAND-OBSERVED-IDS" || s.ID == "MSG-FAILED-OBSERVED-IDS" || s.ID == "MSG-COMMAND-PARTS" {
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
	if s.ID == "WEB-AGENT-OAUTH-CIMD" {
		return runAgentAccess(ctx, s, level, runAgentOAuthCIMD)
	}
	if s.ID == "WEB-AGENT-OAUTH" {
		return runAgentAccess(ctx, s, level, runAgentOAuth)
	}
	if s.ID == "WEB-AGENT-LEVELS" {
		return runAgentAccess(ctx, s, level, runAgentLevels)
	}
	if s.Feature == "recordings.read" {
		return runRecordings(ctx, s, level)
	}
	if s.Feature == "phone.read" {
		return runPhone(ctx, s, level)
	}
	if s.ID == "CONSENT-CONNECTOR" {
		return runAgentAccess(ctx, s, level, runConsent)
	}
	if s.Feature == "web.frontdoor" || s.Feature == "web.agents" {
		return runWebServer(ctx, s, level)
	}
	if s.Feature == "mac.extras" {
		return runMacExtras(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "MAC-") {
		return runMac(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "USE-") {
		return runUsage(ctx, s, level)
	}
	if s.ID == "REM-DST" {
		return runDateFixtures(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "CAL-") {
		return runCalendarScenario(ctx, s, level)
	}
	if strings.HasPrefix(s.ID, "NOTE-") {
		return runNotes(ctx, s, level)
	}
	if s.ID == "CON-RELAY" {
		return runRelayWorld(ctx, s, level, true)
	}
	if s.ID == "CON-ME" || s.ID == "CON-NOTE" {
		return runContactsIdentity(ctx, s, level)
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
	return runBinaryScenario(ctx, s, level)
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
func (e Evidence) MarshalJSON() ([]byte, error) {
	type value Evidence
	if e.Impl == "" {
		e.Impl = "go"
	}
	return json.Marshal(value(e))
}

func Suite(ctx context.Context, paths []string, level string, asJSON bool) error {
	if level != "sim" && level != "replica" && level != "real" && level != "web" {
		return fmt.Errorf("level unsupported by tracer")
	}
	scenarios, err := loadLevel(paths, level)
	if err != nil {
		return err
	}
	if len(scenarios) == 0 {
		return fmt.Errorf("no scenarios")
	}
	packagingOnly := true
	for _, scenario := range scenarios {
		directShortcut := scenario.ID == "SC-RUN-REAL" || scenario.Feature == "clock.pack"
		packagingOnly = packagingOnly && (scenario.ID == "APP-SHELL-SIM" && level == "sim" || directShortcut || level == "real" && (scenario.ID == "CONN-CLAUDE-AI" || scenario.ID == "CONN-CHATGPT") || isIPhoneRealScenario(scenario.ID, level) || level == "real" && isIPhoneSensorReal(scenario.ID) || scenario.Feature == "iphone.location" || scenario.Feature == "iphone.health" || scenario.Feature == "packaging.app" || scenario.Feature == "packaging.release" || scenario.Feature == "packaging.setup" || scenario.Feature == "core.shortcuts" || scenario.Feature == "core.permissions" || scenario.Feature == "usage.action_probe" || scenario.Feature == "helper.siri_intents")
	}
	if !packagingOnly && !(len(scenarios) == 1 && scenarios[0].ID == "HELPER-TCC-READY") {
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
		if s.ID == "HELPER-TCC-READY" {
			duration = 10 * time.Minute
		}
		if s.ID == "QUERY-AGENT" {
			duration = 5 * time.Minute
		}
		if s.ID == "REM-SECTIONS" || s.ID == "WEB-REM-PARITY" {
			duration = 6 * time.Minute
		}
		if level == "real" && slices.Contains([]string{"FOC-SET-REAL", "FOC-STATUS-REAL", "MAC-NOTIFY-REAL"}, s.ID) {
			duration = 6 * time.Minute
		}
		if level == "real" && (s.ID == "CONN-CLAUDE-AI" || s.ID == "CONN-CHATGPT" || s.ID == "SEC-REMOTE-ACCESS" || s.Feature == "mail.commands" || s.ID == "MAIL-BODY-REAL") {
			duration = 6 * time.Minute
		}
		if isIPhoneRealScenario(s.ID, level) || level == "real" && isIPhoneSensorReal(s.ID) {
			duration = 70 * time.Minute
		}
		if s.ID == "RELAY-STREAMS" {
			duration = 12 * time.Minute
		}
		if s.ID == "RELAY-REAL" || s.ID == "CONN-RELAY-CLAUDE" || s.ID == "CONN-CLAUDE-CODE" {
			duration = 10 * time.Minute
		}
		if s.ID == "CALL-IDLE" && level == "sim" {
			duration = 20 * time.Minute
		}
		if s.Feature == "vision.queries" && level == "real" {
			duration = 20 * time.Minute
		}
		if s.Feature == "phone.read" || s.Feature == "recordings.read" {
			duration = 45 * time.Minute
		}
		if s.Feature == "maps.queries" && level == "real" {
			duration = 20 * time.Minute
			if s.ID == "MAP-IDLE" {
				duration = 35 * time.Minute
			}
		}
		if s.Feature == "safari.capture" {
			duration = 60 * time.Minute
		}
		if s.ID == "RELAY-STREAMS" {
			duration = 12 * time.Minute
		}
		if s.ID == "NOTE-EXTERNAL" && level != "sim" {
			duration = 5 * time.Minute
		}
		if level == "real" && slices.Contains([]string{"NOTE-CRUD", "NOTE-EXTRAS", "NOTE-EDIT-PARITY"}, s.ID) {
			duration = 7 * time.Minute
		}
		if s.ID == "CORE-SEARCH-UPDATE-PERF" {
			duration = 10 * time.Minute
		}
		if level == "real" && (s.ID == "CORE-LIVE-LATENCY" || s.ID == "REM-CRUD" || s.ID == "WEB-REM-WRITE") {
			duration = 8 * time.Minute
		}
		if (s.Feature == "web.relay" || s.ID == "CON-RELAY") && level == "replica" {
			duration = 5 * time.Minute
		}
		if s.ID == "WEB-RELAY-REPLICA" {
			duration = 10 * time.Minute
		}
		if s.Feature == "web.readmodels" || s.Feature == "web.messages.read" {
			duration = 15 * time.Minute
		}
		if level == "real" && (s.Feature == "messages.actions" || s.Feature == "web.media") {
			duration = 8 * time.Minute
		}
		if (strings.HasPrefix(s.ID, "CAL-") || s.ID == "WEB-CAL-WRITE") && level != "sim" {
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
		if s.ID == "CORE-QUERY-HELPER" || s.ID == "HELPER-SIGNED-APP-CACHE" || s.ID == "HELPER-RELEASE-COMMANDS" {
			duration = 10 * time.Minute
		}
		if s.ID == "HELPER-SESSION-SOAK" {
			duration = 12 * time.Minute
		}
		if level == "web" && s.Web != nil {
			duration = time.Duration(max(1, len(s.Web.Browsers)*len(s.Web.Themes)*len(s.Web.Layouts))) * webRunBudget(s)
		}
		if (strings.HasPrefix(s.ID, "MAC-")) && level == "real" {
			duration = 45 * time.Minute
		}
		if level == "sim" && slices.Contains([]string{"MSG-DECODE", "MSG-DECODE-BALLOON", "MSG-DECODE-FORMATTING", "MSG-READ-RICH-FIELDS", "MSG-DELETE-VISIBILITY", "MSG-WATCHER-REARM", "MSG-MEDIA-CLEANUP"}, s.ID) {
			duration = 45 * time.Minute
		}
		if s.ID == "SC-ACTIONS" || s.ID == "SIRI-INTENTS" {
			duration = 30 * time.Minute
		}
		if s.Feature == "files.commands" && level != "sim" || s.ID == "SC-IDLE" && level == "real" {
			duration = 20 * time.Minute
		}
		if s.ID == "FILE-IDLE" && level == "real" {
			duration = 7 * time.Minute
		}
		if s.ID == "CAP-SIM" {
			duration = 20 * time.Minute
		}
		if level == "real" && slices.Contains([]string{"MSG-SELFTEST", "MSG-READSTATE-REAL", "MSG-DELETE-REAL", "MSG-RESEND", "MSG-SEND-LATER", "MSG-MENTION-SEND", "MSG-AUDIO-MESSAGE", "WEB-MEDIA-VARIANTS", "WEB-UPLOAD-NATIVE-STAGING", "WEB-MSG-REAL-READ"}, s.ID) {
			duration = 6 * time.Minute
		}
		if level == "real" && slices.Contains([]string{"SC-RUN-REAL", "CLK-ALARM-REAL", "CLK-TIMER-REAL", "SIRI-INTENTS", "USE-ACTION-PROBE"}, s.ID) {
			duration = 7 * time.Minute
		}
		bounded, cancel := context.WithTimeout(ctx, duration)
		secretBefore, secretWasSet := os.LookupEnv("APPLE_BRIDGE_SECRET_STORE")
		e := Run(bounded, s, level)
		if level == "sim" {
			if e.Conditions == nil {
				e.Conditions = map[string]string{}
			}
			e.Conditions["simapple_impl"] = "go"
			if binary := os.Getenv("AB_LAB_SIMAPPLE"); binary != "" {
				e.Conditions["simapple_impl"] = "rust"
				e.Conditions["simapple_binary"] = binary
			}
		}
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
