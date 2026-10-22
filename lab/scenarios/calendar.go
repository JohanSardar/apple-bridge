package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCalendarScenario(ctx context.Context, s Scenario, level string) Evidence {
	if s.ID == "CAL-INVALIDATED-EVENT" || s.ID == "CAL-HINT-COVERAGE" {
		return runCalendarInvalidation(ctx, s, level)
	}
	if s.ID == "CAL-STAMP-FIELDS" {
		return runCalendarStampFields(ctx, s, level)
	}
	if s.ID == "CAL-PERIODIC-VERIFICATION" {
		return runCalendarVerification(ctx, s, level)
	}
	if s.ID == "CAL-LARGE-CAPTURE" {
		return runCalendarLargeCapture(ctx, s, level)
	}
	scenarioID := s.ID
	switch s.ID {
	case "CAL-SPLIT-ATOMIC-ALLDAY":
		s.ID = "CAL-ALLDAY-FUTURE"
	}
	e := runCalendarProtocol(ctx, s, level, "rest")
	e.Scenario = scenarioID
	if s.ID == "CAL-CRUD" && e.Passed {
		m := runCalendarProtocol(ctx, s, level, "mcp")
		e.Conditions["mcp_run_id"] = m.RunID
		for _, a := range m.Assertions {
			a.Name = "mcp_" + a.Name
			e.Assertions = append(e.Assertions, a)
		}
		for k, v := range m.Latency {
			e.Latency["mcp_"+k] = v
		}
		for k, v := range m.Conditions {
			e.Conditions["mcp_"+k] = v
		}
		e.Passed = e.Passed && m.Passed
	}
	return e
}
func runCalendarProtocol(ctx context.Context, s Scenario, level, protocol string) (result Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{Conditions: calendarHostConditions(), RunID: contracts.NewID("cal"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level == "replica" {
		e.Hosts = append(e.Hosts, "ubuntu")
	}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	var diagnose func()
	fail := func(name string, err error) Evidence {
		if diagnose != nil {
			diagnose()
		}
		add(name, false)
		if err != nil {
			fmt.Fprintln(os.Stderr, "calendar:", name, err)
		}
		return e
	}
	tag := "[ab-test " + e.RunID + "]"
	title := tag + " event"
	calendarNative := "test-calendar"
	var w *world.World
	var err error
	if level == "sim" {
		w, err = world.New(ctx, level, 1, 0)
	} else {
		setup := world.NativeSetup{}
		if s.ID == "CAL-EXCEPTION-INITIAL-CAPTURE" {
			setup.BeforeStart = func(nativeWorld *world.World) error {
				result, err := nativeWorld.Truth("fixture", map[string]any{"source": "calendar", "action": "create_calendar", "tag": tag})
				if err != nil {
					return err
				}
				calendarNative, _ = result["id"].(string)
				_, err = nativeWorld.Truth("fixture", map[string]any{"source": "calendar", "action": "create", "tag": tag, "calendar_id": calendarNative, "title": title, "start": "2026-11-01T05:30:00Z", "end": "2026-11-01T07:30:00Z", "time_zone": "America/New_York", "recurrence": []any{map[string]any{"frequency": "daily", "interval": 1, "count": 3}}})
				if err != nil {
					return err
				}
				_, err = nativeWorld.Truth("fixture", map[string]any{"source": "calendar", "action": "modify", "tag": tag, "calendar_id": calendarNative, "index": 1, "new_title": tag + " initial exception"})
				return err
			}
			setup.Cleanup = func(nativeWorld *world.World) error {
				_, err := nativeWorld.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup", "tag": tag})
				return err
			}
		}
		w, err = world.NewRealWithSetup(ctx, level, 1, setup)
	}
	if err != nil {
		return fail("world_start", err)
	}
	defer w.Close()
	defer func() {
		if level == "sim" {
			return
		}
		private := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev/cal-fix-traces")
		if os.MkdirAll(private, 0700) != nil {
			return
		}
		for _, name := range []string{"gateway/helper/capture-trace.jsonl", "gateway/capture-trace.jsonl"} {
			if bytes, err := os.ReadFile(filepath.Join(w.Dir, name)); err == nil {
				if name == "gateway/helper/capture-trace.jsonl" {
					calendarScanWaitEvidence(&result, bytes)
				}
				os.WriteFile(filepath.Join(private, e.RunID+"-"+strings.ReplaceAll(name, "/", "_")), bytes, 0600)
			}
		}
	}()
	e.NetworkPath = w.NetworkPath
	diagnose = func() {
		e.Metrics = map[string]any{}
		all, readErr := w.Records(w.Gateway)
		states := []map[string]any{}
		for _, r := range all {
			if r.Collection == "system.source" {
				var d map[string]any
				json.Unmarshal(r.Data, &d)
				states = append(states, map[string]any{"source": d["source"], "state": d["state"], "reason": d["reason"], "capabilities": d["capabilities"]})
			}
		}
		e.Metrics["source_readiness"] = states
		e.Metrics["records_read_succeeded"] = readErr == nil
		if level == "sim" {
			return
		}
		truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag, "calendar_id": calendarNative})
		rows, _ := truth["records"].([]any)
		e.Metrics["truth_fixture_count"] = len(rows)
		e.Metrics["truth_read_succeeded"] = truthErr == nil
		private := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev/lab-failures", filepath.Base(w.Dir), "calendar")
		if os.MkdirAll(private, 0700) == nil {
			for _, name := range []string{"gateway.log", "gateway/capture-trace.jsonl", "gateway/helper/capture-trace.jsonl", "gateway/helper/calendar-baseline.json", "gateway/helper/calendar-lookups.json"} {
				if bytes, err := os.ReadFile(filepath.Join(w.Dir, name)); err == nil {
					os.WriteFile(filepath.Join(private, strings.ReplaceAll(name, "/", "_")), bytes, 0600)
				}
			}
		}
	}

	if level != "sim" {
		add("private_signed_apps_per_world", strings.HasPrefix(w.HelperApp, w.Dir+"/apps/") && strings.HasPrefix(w.TruthApp, w.Dir+"/apps/"))
	}

	raw := func(r contracts.Record) map[string]any { var d map[string]any; json.Unmarshal(r.Data, &d); return d }
	records := func(endpoint world.Endpoint) []contracts.Record {
		all, _ := w.Records(endpoint)
		out := []contracts.Record{}
		for _, r := range all {
			if r.Collection == "calendar.event" && strings.Contains(recordTitle(r), tag) {
				out = append(out, r)
			}
		}
		slices.SortFunc(out, func(a, b contracts.Record) int {
			return strings.Compare(fmt.Sprint(raw(a)["start"]), fmt.Sprint(raw(b)["start"]))
		})
		return out
	}

	calendar := func() contracts.Record {
		aliases, _ := gatewayAliases(w)
		wanted := aliases[aliasKey(contracts.Calendar, "calendar.calendar", calendarNative)]
		all, _ := w.Records(w.Gateway)
		for _, r := range all {
			if r.Collection == "calendar.calendar" && recordTitle(r) == map[bool]string{true: "Bridge Test", false: "Bridge Test " + tag}[level == "sim"] && r.ID == wanted {
				return r
			}
		}
		return contracts.Record{}
	}
	nativeEffects := map[string]int64{}
	phaseEffects := []any{}
	simEffects := map[string]time.Time{}
	fixture := func(action, key string, data map[string]any) (map[string]any, error) {
		if level == "sim" {
			var response map[string]any
			err := w.ControlResult("/calendar", map[string]any{"action": action, "key": key, "data": data}, &response)
			effect, _ := time.Parse(time.RFC3339Nano, fmt.Sprint(response["effect_at"]))
			simEffects[fmt.Sprint(data["title"])] = effect
			return response, err
		}
		request := map[string]any{"source": "calendar", "action": action, "tag": tag}
		if (action != "create_calendar" || s.ID == "CAL-EXCEPTION-INITIAL-CAPTURE") && calendarNative != "" {
			request["calendar_id"] = calendarNative
		}
		for k, v := range data {
			request[k] = v
		}
		response, err := w.Truth("fixture", request)
		if response != nil {
			if ns, ok := response["effect_uptime_ns"].(json.Number); ok {
				nativeEffects[fmt.Sprint(data["title"])], _ = ns.Int64()
				if action == "create" {
					id, _ := response["id"].(string)
					phaseEffects = append(phaseEffects, map[string]any{"collection": "calendar.event", "title": data["title"], "key": fmt.Sprintf("%x", sha256.Sum256([]byte(id))), "effect_uptime_ns": ns, "effect_wall_ns": response["effect_wall_ns"]})
				}
			}
		}
		return response, err
	}
	if level != "sim" {
		if _, err = w.Truth("observe", map[string]any{"source": "calendar", "tag": tag}); err != nil {
			return fail("independent_truth_access", err)
		}
		defer func() {
			clean := false
			for attempt := 0; attempt < 8; attempt++ {
				_, cleanupErr := w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup", "tag": tag})
				audit, auditErr := w.RecoverTruth("observe", map[string]any{"source": "calendar", "action": "audit", "tag": tag})
				remaining, _ := audit["records"].([]any)
				if cleanupErr != nil || auditErr != nil {
					break
				}
				if len(remaining) == 0 {
					_, cleanupErr = w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup_run_calendar", "tag": tag})
					clean = cleanupErr == nil
					break
				}
			}
			result.Assertions = append(result.Assertions, Assertion{Name: "tagged_fixture_cleanup_audit", Passed: clean})
			result.Passed = result.Passed && clean
		}()
	}
	if level != "sim" {
		initialComplete := func() bool {
			bytes, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"))
			return strings.Contains(string(bytes), "calendar_scan_complete")
		}
		if err = w.Await(90*time.Second, initialComplete); err != nil {
			return fail("initial_calendar_scan_complete", err)
		}
		add("initial_calendar_scan_complete", true)
	}
	if level == "sim" {
		_, err = fixture("calendar", "test-calendar", nil)
	} else {
		var calendarResult map[string]any
		calendarResult, err = fixture("create_calendar", "", nil)
		if calendarResult != nil {
			calendarNative, _ = calendarResult["id"].(string)
		}
	}
	if err != nil {
		return fail("fixture_calendar", err)
	}
	if err = w.Await(10*time.Second, func() bool { return calendar().ID != "" }); err != nil {
		return fail("calendar_visible", err)
	}
	add("calendar_visible", true)
	if level != "sim" {
		truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag, "calendar_id": calendarNative})
		entries, _ := truth["calendars"].([]any)
		matches := false
		for _, entry := range entries {
			expected, ok := entry.(map[string]any)
			if !ok || expected["id"] != calendarNative {
				continue
			}
			actual := raw(calendar())
			matches = true
			for _, field := range []string{"title", "writable", "account", "color", "type"} {
				a, _ := json.Marshal(expected[field])
				b, _ := json.Marshal(actual[field])
				matches = matches && string(a) == string(b)
			}
		}
		add("truth_calendar_fields", truthErr == nil && matches)
	}
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	observers := []*watcher{}
	for _, endpoint := range endpoints {
		observer, err := watch(ctx, endpoint, false, func(r contracts.Record) bool {
			if r.Collection != "calendar.event" || !strings.Contains(recordTitle(r), tag) {
				return true
			}
			if level == "sim" {
				return truthAbsent(w, r)
			}
			truth, err := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag, "title": recordTitle(r)})
			rows, _ := truth["records"].([]any)
			aliases, aliasErr := gatewayAliases(w)
			if err != nil || aliasErr != nil {
				return false
			}
			for _, row := range rows {
				if d, ok := row.(map[string]any); ok && aliases[aliasKey(contracts.Calendar, "calendar.event", calendarTruthNativeKey(d))] == r.ID {
					return false
				}
			}
			return true
		})
		if err != nil {
			return fail("sse_observer", err)
		}
		defer observer.close()
		observers = append(observers, observer)
	}
	seen := func(observer *watcher, title string) (Seen, bool) {
		observer.mu.Lock()
		defer observer.mu.Unlock()
		for _, x := range observer.seen {
			if x.Change.Record.Collection == "calendar.event" && recordTitle(x.Change.Record) == title && x.Change.Op == "upsert" {
				return x, true
			}
		}
		return Seen{}, false
	}
	calendarLatencies := map[string][]float64{}
	latency := func(name string, effect time.Time) bool {
		if w.Await(3*time.Second, func() bool {
			for _, observer := range observers {
				if _, ok := seen(observer, name); !ok {
					return false
				}
			}
			return true
		}) != nil {
			return false
		}
		for i, observer := range observers {
			x, _ := seen(observer, name)
			key := "effect_done→gateway_visible"
			if i > 0 {
				key = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
			}
			if saved, ok := simEffects[name]; ok {
				effect = saved
			}
			if level == "sim" && (effect.IsZero() || x.At.Before(effect)) {
				return false
			}
			value := float64(x.At.Sub(effect)) / float64(time.Millisecond)
			if level != "sim" && nativeEffects[name] <= 0 {
				return false
			}
			if ns := nativeEffects[name]; ns > 0 {
				if x.UptimeNS < ns {
					return false
				}
				value = float64(x.UptimeNS-ns) / float64(time.Millisecond)
			}
			old := e.Latency[key]
			values := []float64{value}
			if old.Samples > 0 {
				values = append(calendarLatencies[key], value)
			}
			calendarLatencies[key] = values
			e.Latency[key] = stats(values, 0)
		}
		return true
	}
	var last contracts.CommandRecord
	commandLatencies := map[string][]float64{}
	var commandVisibility func(string, map[string]any)
	submit := func(op string, args map[string]any, version uint64) error {
		var err error
		endpoint := w.Gateway
		if level == "replica" {
			endpoint = w.Replicas[0]
		}
		client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
		last = contracts.CommandRecord{}
		commandID := contracts.NewCommandID()
		started := time.Now()
		err = client.Call(ctx, op, contracts.Command{ID: commandID, Args: mustJSON(args), ExpectedVersion: version, WaitMS: 15000}, &last)
		if err != nil {
			_, _ = world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: commandID}, &last)
		}
		if err == nil && last.Disposition == "" {
			err = w.Await(45*time.Second, func() bool {
				_, statusErr := world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: commandID}, &last)
				return statusErr == nil && last.Disposition != ""
			})
		}
		elapsed := time.Since(started)
		if level != "sim" {
			private := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev/cal-fix", e.RunID)
			os.MkdirAll(private, 0700)
			truth, _ := w.Truth("observe", map[string]any{"source": "calendar", "action": "audit", "tag": tag, "calendar_id": calendarNative})
			bytes, _ := json.MarshalIndent(map[string]any{"bridge": records(w.Gateway), "truth": truth, "command": last}, "", "  ")
			os.WriteFile(filepath.Join(private, commandID+".json"), bytes, 0600)
		}
		if err == nil && last.Disposition == "locally_observed" {
			if commandVisibility != nil {
				commandVisibility(op, args)
			}
			commandLatencies[op] = append(commandLatencies[op], float64(elapsed)/float64(time.Millisecond))
			e.Latency["command_submit→"+op+"_observed"] = stats(commandLatencies[op], 0)
		}
		if err == nil && last.Disposition != "locally_observed" {
			return fmt.Errorf("disposition %s", last.Disposition)
		}
		return err
	}
	awaitCount := func(n int) bool {
		return w.Await(5*time.Second, func() bool {
			for _, ep := range endpoints {
				if len(records(ep)) != n {
					return false
				}
			}
			return true
		}) == nil
	}
	commandVisibility = func(operation string, args map[string]any) {
		if level == "sim" || s.ID != "CAL-CRUD" || (operation != "calendar.create_event" && operation != "calendar.update_event") {
			return
		}
		name, _ := args["title"].(string)
		ns := calendarCommandEffect(w, last, operation)
		add("command_native_effect_clock", ns > 0 && name != "")
		if ns <= 0 || name == "" {
			return
		}
		nativeEffects[name] = ns
		add("command_capture_visible", latency(name, time.Time{}))
	}
	start, end := "2026-11-01T05:30:00Z", "2026-11-01T07:30:00Z"
	allDay := s.ID == "CAL-ALLDAY" || s.ID == "CAL-ALLDAY-FUTURE"
	if allDay {
		start, end = "2026-11-01", "2026-11-03"
	}
	event := map[string]any{"title": title, "start": start, "end": end, "all_day": allDay, "time_zone": "America/New_York"}
	recurring := s.ID == "CAL-RECUR-THIS" || s.ID == "CAL-RECUR-FUTURE" || s.ID == "CAL-MOVED-OCCURRENCE" || s.ID == "CAL-MOVED-OUTSIDE-WINDOW" || s.ID == "CAL-ALLDAY-FUTURE" || s.ID == "CAL-EXCEPTION-INITIAL-CAPTURE" || s.ID == "CAL-SPLIT-FRAME-CAP"
	if recurring {
		event["recurrence"] = []any{map[string]any{"frequency": "daily", "interval": 1, "count": 3}}
		if s.ID == "CAL-SPLIT-FRAME-CAP" {
			event["recurrence"] = []any{map[string]any{"frequency": "daily", "interval": 1, "count": 400}}
		}
	}
	if s.ID == "CAL-CRUD" {
		event["notes"] = tag + " notes"
		event["location"] = tag + " location"
		event["url"] = "https://example.com/calendar"
		event["availability"] = 1
		event["alarms"] = []any{map[string]any{"relative_offset": -600}}
		event["calendar_id"] = calendar().ID
		if err = submit("calendar.create_event", event, 0); err != nil {
			return fail("create_event", err)
		}
		add("create_event_observed", last.CommitSeq > 0)
	} else if s.ID != "CAL-EXCEPTION-INITIAL-CAPTURE" {
		if _, err = fixture("create", "series", event); err != nil {
			return fail("external_create", err)
		}
	}
	effectDone := time.Now()
	count := 1
	if recurring {
		count = 3
	}
	if s.ID == "CAL-SPLIT-FRAME-CAP" {
		count = 400
	}
	if !awaitCount(count) {
		return fail("initial_occurrences_visible", nil)
	}
	add("initial_occurrences_visible", true)
	if s.ID != "CAL-CRUD" && s.ID != "CAL-EXCEPTION-INITIAL-CAPTURE" && !latency(title, effectDone) {
		return fail("external_capture_latency", nil)
	}
	baseline := records(w.Gateway)
	var agenda contracts.ReadResult
	agendaClient := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
	agendaErr := agendaClient.Call(ctx, "calendar.agenda", contracts.AgendaQuery{Start: "2026-11-01T00:00:00Z", End: "2026-11-04T12:00:00Z", CalendarID: calendar().ID}, &agenda)
	agendaCount := count
	if s.ID == "CAL-SPLIT-FRAME-CAP" {
		agendaCount = 4
	}
	add("agenda_returns_occurrences", agendaErr == nil && len(agenda.Records) == agendaCount)

	switch s.ID {
	case "CAL-EXCEPTION-INITIAL-CAPTURE":
		found := false
		for _, r := range baseline {
			if recordTitle(r) == tag+" initial exception" {
				d := raw(r)
				found = d["is_exception"] == true && d["recurrence"] != nil && d["occurrence_date"] != nil && d["calendar_id"] == calendar().ID
			}
		}
		add("initial_detached_exception_has_native_series_rules", found)
	case "CAL-CLEANUP-SAFETY":
		sentinelTag := strings.TrimSuffix(tag, "]") + "-sentinel]"
		sentinel := map[string]any{"source": "calendar", "action": "create", "tag": sentinelTag, "calendar_id": calendarNative, "title": sentinelTag + " outside window", "start": time.Now().AddDate(3, 0, 0).UTC().Format(time.RFC3339Nano), "end": time.Now().AddDate(3, 0, 0).Add(time.Hour).UTC().Format(time.RFC3339Nano)}
		if _, err = w.Truth("fixture", sentinel); err != nil {
			return fail("create_outside_window_sentinel", err)
		}
		defer func() {
			_, cleanupErr := w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup", "tag": sentinelTag, "calendar_id": calendarNative})
			if cleanupErr != nil {
				result.Passed = false
				result.Assertions = append(result.Assertions, Assertion{Name: "sentinel_cleanup", Passed: false})
			}
		}()
		response, cleanupErr := w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup_calendar", "tag": tag, "calendar_id": calendarNative})
		add("whole_calendar_removal_disabled", cleanupErr == nil && response["calendar_retained"] == true && response["all_dates_ownership_proven"] == false)
		audit, auditErr := w.Truth("observe", map[string]any{"source": "calendar", "action": "audit", "tag": sentinelTag, "calendar_id": calendarNative})
		rows, _ := audit["records"].([]any)
		add("outside_window_other_tag_survives", auditErr == nil && len(rows) == 1)
		add("owned_occurrence_removed_individually", awaitCount(0))
	case "CAL-CREATE-EXTERNAL":
		for i := 0; i < 9; i++ {
			copy := map[string]any{}
			for k, v := range event {
				copy[k] = v
			}
			name := fmt.Sprintf("%s sample-%d", tag, i)
			copy["title"] = name
			if _, err = fixture("create", fmt.Sprint("external-", i), copy); err != nil {
				return fail("latency_fixture", err)
			}
			if !latency(name, time.Now()) {
				return fail("latency_sample", nil)
			}
		}
		if level != "sim" {
			nativeLatencyEvidence(w, &e, phaseEffects, observers)
			add("calendar_database_hint_observed", e.Latency["native_save→database_hint_delivery"].Samples > 0)
			add("calendar_native_phase_timestamps", len(e.NativeTimings) == 10 && e.Latency["refetch_duration"].Samples == 10 && e.Latency["capture_sent→gateway_commit"].Samples == 10 && e.Latency["gateway_commit→replica_visible"].Samples == 10)
		}
	case "CAL-CRUD":
		current := records(w.Gateway)[0]
		if err = submit("calendar.update_event", map[string]any{"id": current.ID, "span": "this", "title": tag + " updated", "notes": tag + " changed notes", "location": tag + " changed location", "url": "https://example.com/updated", "availability": 2, "alarms": []any{map[string]any{"relative_offset": -300}}}, current.Version); err != nil {
			return fail("update_event", err)
		}
		if !awaitCount(1) {
			return fail("updated_visible", nil)
		}
		add("update_preserves_bridge_id", records(w.Gateway)[0].ID == current.ID)
		fields := raw(records(w.Gateway)[0])
		alarms, _ := fields["alarms"].([]any)
		alarmMatches := false
		if len(alarms) == 1 {
			alarm, _ := alarms[0].(map[string]any)
			alarmMatches = alarm["relative_offset"] == float64(-300)
		}
		add("event_fields_and_alarm_updated", fields["notes"] == tag+" changed notes" && fields["location"] == tag+" changed location" && fields["url"] == "https://example.com/updated" && fields["availability"] == float64(2) && alarmMatches)
		current = records(w.Gateway)[0]
		if level != "sim" {
			truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag})
			values, _ := truth["records"].([]any)
			matches := false
			if len(values) == 1 {
				d, _ := values[0].(map[string]any)
				actual := raw(current)
				matches = calendarTruthEqual(d, actual)
			}
			add("truth_updated_event_fields", truthErr == nil && matches)
		}
		var stale contracts.CommandRecord
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		staleErr := client.Call(ctx, "calendar.update_event", contracts.Command{Args: mustJSON(map[string]any{"id": current.ID, "span": "this", "title": tag + " stale"}), ExpectedVersion: current.Version - 1, WaitMS: 15000}, &stale)
		var conflict *contracts.Error
		add("stale_version_rejected", errors.As(staleErr, &conflict) && conflict.Code == "conflict" && conflict.Current != nil && conflict.Current.ID == current.ID)
		if err = submit("calendar.delete_event", map[string]any{"id": current.ID, "span": "this"}, current.Version); err != nil {
			return fail("delete_event", err)
		}
		add("delete_absence_visible", awaitCount(0))
		recurringArgs := map[string]any{"calendar_id": calendar().ID, "title": tag + " created series", "start": start, "end": end, "time_zone": "America/New_York", "recurrence": []any{map[string]any{"frequency": "daily", "interval": 1, "count": 3}}}
		if err = submit("calendar.create_event", recurringArgs, 0); err != nil {
			return fail("create_recurring_command", err)
		}
		if !awaitCount(3) {
			return fail("created_series_occurrences", nil)
		}
		if level != "sim" {
			truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag})
			values, _ := truth["records"].([]any)
			add("truth_created_series_occurrences", truthErr == nil && len(values) == 3)
		}
		created := records(w.Gateway)[0]
		if err = submit("calendar.delete_event", map[string]any{"id": created.ID, "span": "future"}, created.Version); err != nil {
			return fail("delete_created_series", err)
		}
		add("created_series_deleted_with_future_span", awaitCount(0))
		if err = submit("calendar.create_calendar", map[string]any{"title": "Bridge Test " + tag}, 0); err != nil {
			return fail("create_calendar", err)
		}
		add("create_calendar_observed", last.CommitSeq > 0)
		if level != "sim" {
			var evidence struct {
				NativeKey   string `json:"native_key"`
				Fingerprint string `json:"fingerprint"`
			}
			if json.Unmarshal(last.Evidence, &evidence) != nil || evidence.NativeKey == "" {
				return fail("new_calendar_identity", nil)
			}
			registration, registrationErr := w.Truth("fixture", map[string]any{"source": "calendar", "action": "record_created_calendar", "calendar_id": evidence.NativeKey, "creation_fingerprint": evidence.Fingerprint, "tag": tag})
			if registrationErr != nil || registration["ownership_recorded"] != true {
				return fail("created_calendar_ownership_durable", registrationErr)
			}
			add("created_calendar_ownership_durable", true)
			defer func() {
				if _, err := w.RecoverTruth("fixture", map[string]any{"source": "calendar", "action": "cleanup_calendar", "calendar_id": evidence.NativeKey, "tag": tag}); err != nil {
					result.Passed = false
					result.Assertions = append(result.Assertions, Assertion{Name: "created_calendar_cleanup", Passed: false})
				}
			}()
		}
	case "CAL-RECUR-THIS", "CAL-MOVED-OCCURRENCE", "CAL-MOVED-OUTSIDE-WINDOW":
		target := baseline[1]
		args := map[string]any{"id": target.ID, "span": "this", "title": tag + " exception"}
		if s.ID == "CAL-MOVED-OCCURRENCE" || s.ID == "CAL-MOVED-OUTSIDE-WINDOW" {
			aliases, _ := gatewayAliases(w)
			native := ""
			for alias, id := range aliases {
				prefix := "calendar/calendar.event/"
				if id == target.ID && strings.HasPrefix(alias, prefix) {
					native = strings.TrimPrefix(alias, prefix)
				}
			}
			movedStart := "2026-11-02T08:30:00Z"
			if s.ID == "CAL-MOVED-OUTSIDE-WINDOW" {
				movedStart = time.Now().AddDate(3, 0, 0).UTC().Format(time.RFC3339Nano)
			}
			move := map[string]any{"index": 1, "new_title": tag + " exception", "start": movedStart}
			if s.ID == "CAL-MOVED-OCCURRENCE" {
				move["end"] = "2026-11-02T10:30:00Z"
			}
			_, err = fixture("move", native, move)
		} else {
			err = submit("calendar.update_event", args, target.Version)
		}
		if err != nil {
			return fail("update_occurrence", err)
		}
		if s.ID == "CAL-MOVED-OUTSIDE-WINDOW" {
			add("moved_outside_window_evicted", awaitCount(2))
			add("moved_outside_window_never_deleted", w.Await(3*time.Second, func() bool {
				for _, observer := range observers {
					observer.mu.Lock()
					evicted := false
					deleted := false
					for _, x := range observer.seen {
						if x.Change.Record.ID == target.ID {
							evicted = evicted || (x.Change.Op == "evict" && x.Change.Record.TombstoneReason == "window_evicted")
							deleted = deleted || x.Change.Op == "delete"
						}
					}
					observer.mu.Unlock()
					if !evicted || deleted {
						return false
					}
				}
				return true
			}) == nil)
			siblings := 0
			for _, r := range records(w.Gateway) {
				for _, before := range baseline {
					if before.ID != target.ID && r.ID == before.ID && reflect.DeepEqual(r.Data, before.Data) {
						siblings++
					}
				}
			}
			add("outside_window_move_preserves_siblings", siblings == 2)
			if level != "sim" {
				audit, auditErr := w.Truth("observe", map[string]any{"source": "calendar", "action": "audit", "tag": tag, "title": tag + " exception", "calendar_id": calendarNative})
				rows, _ := audit["records"].([]any)
				exists := auditErr == nil && len(rows) == 1
				if exists {
					row, _ := rows[0].(map[string]any)
					start, _ := time.Parse(time.RFC3339Nano, fmt.Sprint(row["start"]))
					exists = start.After(time.Now().AddDate(2, 0, 0))
				}
				add("native_occurrence_still_exists_outside_window", exists)
			}
			break
		}
		if !awaitCount(3) {
			return fail("three_occurrences_after_exception", nil)
		}
		add("exception_capture_visible", w.Await(5*time.Second, func() bool {
			for _, r := range records(w.Gateway) {
				if r.ID == target.ID {
					return recordTitle(r) == tag+" exception" && raw(r)["is_exception"] == true
				}
			}
			return false
		}) == nil)
		after := records(w.Gateway)
		kept, unchanged := false, 0
		for _, r := range after {
			if r.ID == target.ID {
				add("exception_has_scoped_native_recurrence", raw(r)["recurrence"] != nil)
				kept = raw(r)["occurrence_date"] == raw(target)["occurrence_date"] && raw(r)["is_exception"] == true && recordTitle(r) == tag+" exception"
				if s.ID == "CAL-MOVED-OCCURRENCE" {
					add("moved_start_and_end", sameCalendarTime(raw(r)["start"], "2026-11-02T08:30:00Z") && sameCalendarTime(raw(r)["end"], "2026-11-02T10:30:00Z"))
				}
			}
			if r.ID != target.ID {
				for _, before := range baseline {
					if r.ID == before.ID && reflect.DeepEqual(r.Data, before.Data) {
						unchanged++
					}
				}
			}
		}
		add("original_occurrence_identity_preserved", kept)
		add("other_occurrences_unchanged", unchanged == 2)
		if s.ID == "CAL-RECUR-THIS" {
			deleted := baseline[2]
			err = submit("calendar.delete_event", map[string]any{"id": deleted.ID, "span": "this"}, deleted.Version)
			add("delete_one_occurrence_is_observed", err == nil && last.Disposition == "locally_observed" && last.CommitSeq > 0)
			add("delete_one_occurrence_keeps_surviving_series", awaitCount(2))
			remaining := records(w.Gateway)
			preserved := len(remaining) == 2
			for _, record := range remaining {
				preserved = preserved && record.ID != deleted.ID && (record.ID == baseline[0].ID || record.ID == baseline[1].ID)
			}
			add("delete_one_occurrence_preserves_other_identities", preserved)
		}
	case "CAL-SPLIT-FRAME-CAP":
		target := baseline[1]
		err = submit("calendar.update_event", map[string]any{"id": target.ID, "span": "future", "title": tag + " oversized future"}, target.Version)
		add("oversized_split_keeps_capture_progressing", err == nil && last.Disposition == "locally_observed" && awaitCount(400))
		if err == nil {
			target = records(w.Gateway)[1]
			err = submit("calendar.update_event", map[string]any{"id": target.ID, "span": "this", "title": tag + " continued"}, target.Version)
			add("source_captures_after_oversized_split", err == nil && last.Disposition == "locally_observed")
		}
	case "CAL-ALLDAY-FUTURE":
		finishSplit := watchCalendarSplit(ctx, w, tag)
		defer func() {
			if finishSplit != nil {
				finishSplit()
			}
		}()
		target := baseline[1]
		if err = submit("calendar.update_event", map[string]any{"id": target.ID, "span": "future", "start": "2026-11-04", "end": "2026-11-06"}, target.Version); err != nil {
			return fail("all_day_future_update", err)
		}
		add("all_day_future_civil_offsets", w.Await(3*time.Second, func() bool {
			after := records(w.Gateway)
			return len(after) == 3 && raw(after[0])["start"] == "2026-11-01" && raw(after[1])["start"] == "2026-11-04" && raw(after[2])["start"] == "2026-11-05" && raw(after[2])["end"] == "2026-11-07"
		}) == nil)
		after := records(w.Gateway)
		identities := len(after) == 3
		if identities {
			identities = after[0].ID == baseline[0].ID && raw(after[1])["series_id"] != raw(baseline[1])["series_id"] && after[1].ID != baseline[1].ID && after[2].ID != baseline[2].ID
			for _, record := range after[1:] {
				identities = identities && raw(record)["occurrence_date"] == raw(record)["start"]
			}
		}
		add("future_split_replaces_occurrence_identities", identities)
		add("future_split_atomic_on_gateway_and_replica", finishSplit())
		finishSplit = nil
	case "CAL-RECUR-FUTURE":
		exception := baseline[2]
		if err = submit("calendar.update_event", map[string]any{"id": exception.ID, "span": "this", "title": tag + " preserved exception"}, exception.Version); err != nil {
			return fail("seed_exception", err)
		}
		if w.Await(3*time.Second, func() bool {
			for _, r := range records(w.Gateway) {
				if r.ID == exception.ID {
					return recordTitle(r) == tag+" preserved exception"
				}
			}
			return false
		}) != nil {
			return fail("exception_visible", nil)
		}
		finishSplit := watchCalendarSplit(ctx, w, tag)
		defer func() {
			if finishSplit != nil {
				finishSplit()
			}
		}()
		target := records(w.Gateway)[1]
		if err = submit("calendar.update_event", map[string]any{"id": target.ID, "span": "future", "title": tag + " future", "recurrence": []any{map[string]any{"frequency": "daily", "interval": 2, "count": 2}}}, target.Version); err != nil {
			return fail("future_update", err)
		}
		add("new_rule_regenerates_occurrences", awaitCount(4))
		after := records(w.Gateway)
		preserved, generated := false, false
		for _, r := range after {
			d := raw(r)
			preserved = preserved || (recordTitle(r) == tag+" preserved exception" && d["is_exception"] == true && sameCalendarTime(d["start"], raw(exception)["start"]))
			generated = generated || (recordTitle(r) == tag+" future" && sameCalendarTime(d["start"], "2026-11-04T06:30:00Z"))
		}
		add("future_split_preserves_exception", preserved)
		add("future_rule_changes_dates", generated)
		series := map[string]bool{}
		for _, r := range after {
			series[fmt.Sprint(raw(r)["series_id"])] = true
		}
		add("future_span_splits_series", len(series) == 2)
		add("future_split_atomic_on_gateway_and_replica", finishSplit())
		finishSplit = nil
		if len(after) >= 2 {
			target = after[1]
			if err = submit("calendar.delete_event", map[string]any{"id": target.ID, "span": "future"}, target.Version); err != nil {
				return fail("delete_future", err)
			}
			add("delete_future_preserves_past", awaitCount(1) && recordTitle(records(w.Gateway)[0]) == title)
		}
	case "CAL-RECUR-ALL-REJECT":
		target := baseline[0]
		err = submit("calendar.update_event", map[string]any{"id": target.ID, "span": "all", "title": tag + " forbidden"}, target.Version)
		add("span_all_rejected_before_dispatch", errors.Is(err, contracts.ErrUnsupported) && last.Command.ID == "")
		add("rejected_span_preserves_events", reflect.DeepEqual(baseline, records(w.Gateway)))
	case "CAL-DST":
		value := raw(baseline[0])
		a, _ := time.Parse(time.RFC3339Nano, fmt.Sprint(value["start"]))
		b, _ := time.Parse(time.RFC3339Nano, fmt.Sprint(value["end"]))
		zone, _ := time.LoadLocation("America/New_York")
		add("dst_zone_and_elapsed_time", value["time_zone"] == "America/New_York" && a.In(zone).Hour() == 1 && b.In(zone).Hour() == 2 && b.Sub(a) == 2*time.Hour)
	case "CAL-ALLDAY":
		value := raw(baseline[0])
		add("floating_dates_preserved", value["all_day"] == true && value["start"] == "2026-11-01" && value["end"] == "2026-11-03")
	case "CAL-ID-CHURN":
		if _, err = fixture("churn", "", nil); err != nil {
			return fail("churn_fixture", err)
		}
		add("stable_external_alias_identity", w.Await(3*time.Second, func() bool {
			now := records(w.Gateway)
			return len(now) == 1 && now[0].ID == baseline[0].ID && now[0].Version == baseline[0].Version
		}) == nil)
	case "CAL-WINDOW-SLIDE":
		now := time.Now()
		today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
		old := today.AddDate(0, 0, -730)
		future := today.AddDate(0, 0, 730).Add(time.Hour)
		for i, date := range []time.Time{old, future} {
			value := map[string]any{"title": fmt.Sprintf("%s boundary-%d", tag, i), "start": date.Format(time.RFC3339Nano), "end": date.Add(time.Hour).Format(time.RFC3339Nano)}
			if _, err = fixture("create", fmt.Sprint("boundary-", i), value); err != nil {
				return fail("window_fixture", err)
			}
		}
		if !awaitCount(2) {
			return fail("initial_window_boundaries", nil)
		}
		if err = w.ControlAction("/calendar", map[string]any{"action": "clock", "now": today.AddDate(0, 0, 1).Format(time.RFC3339Nano)}); err != nil {
			return fail("slide_clock", err)
		}
		add("sliding_window_membership", awaitCount(2))
		eviction, deleted := false, false
		for _, observer := range observers {
			observer.mu.Lock()
			for _, change := range observer.changes {
				if recordTitle(change.Change.Record) == tag+" boundary-0" {
					eviction = eviction || (change.Change.Op == "evict" && change.Change.Record.TombstoneReason == "window_evicted")
					deleted = deleted || change.Change.Op == "delete"
				}
			}
			observer.mu.Unlock()
		}
		add("window_eviction_is_not_deletion", eviction && !deleted)
	case "CAL-READONLY":
		var readonly contracts.Record
		if level == "sim" {
			if _, err = fixture("calendar", "readonly", map[string]any{"title": tag + " readonly", "writable": false, "type": "subscribed", "account": "test"}); err != nil {
				return fail("readonly_fixture", err)
			}
		} else {
			truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "action": "readonly", "tag": tag})
			if truthErr != nil {
				return fail("readonly_truth", truthErr)
			}
			list, _ := truth["calendars"].([]any)
			if len(list) == 0 {
				e.Status = "unverified"
				return fail("readonly_calendar_available", nil)
			}
		}
		if w.Await(5*time.Second, func() bool {
			all, _ := w.Records(w.Gateway)
			for _, r := range all {
				if r.Collection == "calendar.calendar" && raw(r)["writable"] == false && raw(r)["type"] == "subscribed" {
					readonly = r
					return true
				}
			}
			return false
		}) != nil {
			return fail("readonly_calendar_visible", nil)
		}
		args := map[string]any{"calendar_id": readonly.ID, "title": tag + " rejected", "start": start, "end": end}
		err = submit("calendar.create_event", args, 0)
		dispatched := false
		for _, transition := range last.History {
			dispatched = dispatched || transition.State == "dispatched"
		}
		add("readonly_rejected_before_dispatch", errors.Is(err, contracts.ErrUnsupported) && last.Disposition == "rejected_before_dispatch" && !dispatched)
		add("readonly_has_no_effect", len(records(w.Gateway)) == 1)
	case "CAL-PERMISSION-LOSS":
		if err = w.ControlAction("/source-state", map[string]any{"source": "calendar", "state": "permission_missing"}); err != nil {
			return fail("permission_fault", err)
		}
		if _, err = fixture("delete", "series", nil); err != nil {
			return fail("external_delete_denied", err)
		}
		if err = sleep(ctx, 150*time.Millisecond); err != nil {
			return fail("permission_settle", err)
		}
		add("permission_loss_preserves_records", reflect.DeepEqual(baseline, records(w.Gateway)))
		all, _ := w.Records(w.Gateway)
		missing := false
		for _, r := range all {
			d := raw(r)
			if r.Collection == "system.source" && d["source"] == "calendar" && d["state"] == "permission_missing" {
				missing = true
			}
		}
		add("permission_missing_published", missing)
		if err = w.RestartHelper(); err != nil {
			return fail("restart_while_permission_missing", err)
		}
		if err = sleep(ctx, 200*time.Millisecond); err != nil {
			return fail("permission_restart_settle", err)
		}
		add("permission_loss_survives_restart", reflect.DeepEqual(baseline, records(w.Gateway)))
	}
	if level != "sim" {
		private := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev/cal-fix", e.RunID)
		os.MkdirAll(private, 0700)
		truth, _ := w.Truth("observe", map[string]any{"source": "calendar", "action": "audit", "tag": tag, "calendar_id": calendarNative})
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		trace, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "capture-trace.jsonl"))
		e.Metrics["calendar_capture_splits"] = strings.Count(string(trace), "\"stage\":\"calendar_capture_split\"")
		value := map[string]any{"bridge": records(w.Gateway), "truth": truth, "command": last}
		bytes, _ := json.MarshalIndent(value, "", "  ")
		os.WriteFile(filepath.Join(private, "state.json"), bytes, 0600)
	}
	if s.ID != "CAL-PERMISSION-LOSS" && s.ID != "CAL-SPLIT-FRAME-CAP" {
		if level == "sim" {
			add("independent_simtruth_convergence", w.Await(5*time.Second, func() bool { return nativeConvergence(w) }) == nil)
		} else {
			truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag})
			expected, _ := truth["records"].([]any)
			actual := records(w.Gateway)
			aliases, aliasErr := gatewayAliases(w)
			equal := aliasErr == nil && truthErr == nil && len(expected) == len(actual)
			for _, item := range expected {
				d, ok := item.(map[string]any)
				if !ok {
					equal = false
					continue
				}
				found := false
				for _, r := range actual {
					v := raw(r)
					if aliases[aliasKey(contracts.Calendar, "calendar.event", calendarTruthNativeKey(d))] == r.ID && calendarTruthEqual(d, v) {
						found = true
					}
				}
				equal = equal && found
			}
			add("independent_appletruth_equality", equal)
		}
	}
	add("replica_convergence", w.Await(5*time.Second, func() bool {
		gateway, _ := w.Records(w.Gateway)
		for _, ep := range w.Replicas {
			all, _ := w.Records(ep)
			if !reflect.DeepEqual(withoutCommands(gateway), withoutCommands(all)) {
				return false
			}
		}
		return true
	}) == nil)
	for i, observer := range observers {
		add(fmt.Sprintf("contiguous_feed_%d", i), observer.valid())
		observer.mu.Lock()
		safe := !observer.falseDelete
		observer.mu.Unlock()
		add(fmt.Sprintf("no_false_deletion_%d", i), safe)
	}
	for name, latency := range e.Latency {
		if strings.HasPrefix(name, "effect_done→") {
			add("latency_"+name, latency.P95 < 1000 && latency.Samples > 0)
		}
	}
	for _, ep := range endpoints {
		token, err := w.ScopedToken(ep, "system:read")
		if err != nil {
			return fail("scope_token", err)
		}
		ep.Token = token
		var output contracts.ReadResult
		_, err = world.Request(ctx, ep, "/v1/op/records.query", contracts.ReadQuery{Collection: "calendar.event", IncludeAliases: true}, &output)
		add("no_scope_leak_query", err == nil && len(output.Records) == 0 && len(output.Aliases) == 0)
		client := scripted.Client{Endpoint: ep, Protocol: "mcp"}
		err = client.Call(ctx, "calendar.agenda", contracts.AgendaQuery{Start: "2026-10-01T00:00:00Z", End: "2026-12-01T00:00:00Z"}, &output)
		add("no_scope_leak_mcp_agenda", err == nil && len(output.Records) == 0)
	}
	if s.ID != "CAL-PERMISSION-LOSS" {
		if level != "sim" {
			pidBytes, pidErr := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
			pid := strings.TrimSpace(string(pidBytes))
			numericPID, parseErr := strconv.Atoi(pid)
			if pidErr != nil || parseErr != nil || numericPID <= 1 || exec.Command("kill", "-STOP", pid).Run() != nil {
				return fail("private_helper_paused_for_integrity", pidErr)
			}
			defer exec.Command("kill", "-CONT", pid).Run()
			add("private_helper_paused_for_integrity", true)
		}
		recordIntegrity(&e, w, nil)
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && (a.Passed || a.Status == "unverified")
	}
	return e
}
func mustJSON(value any) json.RawMessage { b, _ := json.Marshal(value); return b }
func sameCalendarTime(a, b any) bool {
	if a == b {
		return true
	}
	as, ok := a.(string)
	bs, other := b.(string)
	if !ok || !other {
		return false
	}
	at, e := time.Parse(time.RFC3339Nano, as)
	bt, f := time.Parse(time.RFC3339Nano, bs)
	return e == nil && f == nil && at.Equal(bt)
}

func calendarHostConditions() map[string]string {
	load, _ := exec.Command("sysctl", "-n", "vm.loadavg").Output()
	swap, _ := exec.Command("sysctl", "-n", "vm.swapusage").Output()
	fields := strings.Fields(strings.Trim(string(load), "{} \n"))
	slots := 0
	entries, _ := os.ReadDir("/tmp")
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), "apple-bridge-heavy-") && strings.HasSuffix(entry.Name(), ".lock") {
			slots++
		}
	}
	return map[string]string{"swap_usage": strings.TrimSpace(string(swap)), "load_average": strings.Join(fields, " "), "heavy_slots_in_use": fmt.Sprint(slots), "clock": "native CLOCK_UPTIME_RAW for real fixtures; simulated native UTC effect timestamp before enqueue", "metadata_endpoint": "Apple effect commit to gateway/local SSE on each replica"}
}

func calendarTruthEqual(truth, actual map[string]any) bool {
	if truth["title"] != actual["title"] || !sameCalendarTime(truth["start"], actual["start"]) || !sameCalendarTime(truth["end"], actual["end"]) || truth["all_day"] != actual["all_day"] {
		return false
	}
	for _, field := range []string{"notes", "location", "url", "availability", "status", "alarms", "attendees", "is_exception"} {
		left, _ := json.Marshal(truth[field])
		right, _ := json.Marshal(actual[field])
		if string(left) != string(right) {
			return false
		}
	}
	expectedZone, _ := truth["time_zone"].(string)
	actualZone, _ := actual["time_zone"].(string)
	if expectedZone != actualZone {
		return false
	}
	if actual["occurrence_date"] != nil {
		series := truth["series_id"]
		if series == nil {
			series = truth["external_id"]
		}
		if series == "" {
			series = truth["id"]
		}
		return actual["series_id"] == series && sameCalendarTime(truth["occurrence_date"], actual["occurrence_date"])
	}
	return true
}

func calendarTruthNativeKey(value map[string]any) string {
	id, _ := value["id"].(string)
	if value["recurring"] == true || value["is_exception"] == true {
		original, _ := value["occurrence_date"].(string)
		if parsed, err := time.Parse(time.RFC3339Nano, original); err == nil {
			original = parsed.UTC().Format("2006-01-02T15:04:05.000Z")
		}
		id += "|" + original
	}
	return id
}
