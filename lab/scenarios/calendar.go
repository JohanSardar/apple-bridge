package scenarios

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCalendarScenario(ctx context.Context, s Scenario, level string) Evidence {
	e := runCalendarProtocol(ctx, s, level, "rest")
	if s.ID == "CAL-CRUD" && e.Passed {
		m := runCalendarProtocol(ctx, s, level, "mcp")
		for _, a := range m.Assertions {
			a.Name = "mcp_" + a.Name
			e.Assertions = append(e.Assertions, a)
		}
		for k, v := range m.Latency {
			e.Latency["mcp_"+k] = v
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
	fail := func(name string, err error) Evidence {
		add(name, false)
		if err != nil {
			fmt.Fprintln(os.Stderr, "calendar:", name, err)
		}
		return e
	}
	var w *world.World
	var err error
	if level == "sim" {
		w, err = world.New(ctx, level, 1, 0)
	} else {
		root, _ := os.Getwd()
		apps := filepath.Join(root, "native/.build/calendar-apps")
		w, err = world.NewRealApps(ctx, level, 1, world.NativeApps{Helper: filepath.Join(apps, "AppleBridgeHelper.app"), Truth: filepath.Join(apps, "AppleTruth.app")})
	}
	if err != nil {
		return fail("world_start", err)
	}
	defer w.Close()
	tag := "[ab-test " + e.RunID + "]"
	title := tag + " event"
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
	calendarNative := "test-calendar"
	calendar := func() contracts.Record {
		aliases, _ := gatewayAliases(w)
		wanted := aliases[aliasKey(contracts.Calendar, "calendar.calendar", calendarNative)]
		all, _ := w.Records(w.Gateway)
		for _, r := range all {
			if r.Collection == "calendar.calendar" && recordTitle(r) == "Bridge Test" && r.ID == wanted {
				return r
			}
		}
		return contracts.Record{}
	}
	nativeEffects := map[string]int64{}
	fixture := func(action, key string, data map[string]any) (map[string]any, error) {
		if level == "sim" {
			return nil, w.ControlAction("/calendar", map[string]any{"action": action, "key": key, "data": data})
		}
		request := map[string]any{"source": "calendar", "action": action, "tag": tag}
		if action != "create_calendar" && calendarNative != "" {
			request["calendar_id"] = calendarNative
		}
		for k, v := range data {
			request[k] = v
		}
		response, err := w.Truth("fixture", request)
		if response != nil {
			if ns, ok := response["effect_uptime_ns"].(json.Number); ok {
				nativeEffects[fmt.Sprint(data["title"])], _ = ns.Int64()
			}
		}
		return response, err
	}
	if level != "sim" {
		if _, err = w.Truth("observe", map[string]any{"source": "calendar", "tag": tag}); err != nil {
			return fail("independent_truth_access", err)
		}
		defer func() {
			if _, cleanupErr := w.Truth("fixture", map[string]any{"source": "calendar", "action": "cleanup", "tag": tag}); cleanupErr != nil {
				result.Passed = false
				result.Assertions = append(result.Assertions, Assertion{Name: "tagged_fixture_cleanup", Passed: false})
			}
		}()
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
			if err != nil {
				return false
			}
			for _, row := range rows {
				if d, ok := row.(map[string]any); ok && sameCalendarTime(d["occurrence_date"], raw(r)["occurrence_date"]) {
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
	var last contracts.CommandRecord
	submit := func(op string, args map[string]any, version uint64) error {
		var err error
		endpoint := w.Gateway
		if level == "replica" {
			endpoint = w.Replicas[0]
		}
		client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
		last = contracts.CommandRecord{}
		commandID := contracts.NewCommandID()
		err = client.Call(ctx, op, contracts.Command{ID: commandID, Args: mustJSON(args), ExpectedVersion: version, WaitMS: 15000}, &last)
		if err != nil {
			_, _ = world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: commandID}, &last)
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
	start, end := "2026-11-01T05:30:00Z", "2026-11-01T07:30:00Z"
	allDay := s.ID == "CAL-ALLDAY"
	if allDay {
		start, end = "2026-11-01", "2026-11-03"
	}
	event := map[string]any{"title": title, "start": start, "end": end, "all_day": allDay, "time_zone": "America/New_York"}
	recurring := s.ID == "CAL-RECUR-THIS" || s.ID == "CAL-RECUR-FUTURE" || s.ID == "CAL-MOVED-OCCURRENCE"
	if recurring {
		event["recurrence"] = []any{map[string]any{"frequency": "daily", "interval": 1, "count": 3}}
	}
	if s.ID == "CAL-CRUD" {
		event["calendar_id"] = calendar().ID
		if err = submit("calendar.create_event", event, 0); err != nil {
			return fail("create_event", err)
		}
		add("create_event_observed", last.CommitSeq > 0)
	} else {
		if _, err = fixture("create", "series", event); err != nil {
			return fail("external_create", err)
		}
	}
	effectDone := time.Now()
	count := 1
	if recurring {
		count = 3
	}
	if !awaitCount(count) {
		return fail("initial_occurrences_visible", nil)
	}
	add("initial_occurrences_visible", true)
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
			value := max(0, float64(x.At.Sub(effect))/float64(time.Millisecond))
			if ns := nativeEffects[name]; ns > 0 {
				value = max(0, float64(x.UptimeNS-ns)/float64(time.Millisecond))
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
	if !latency(title, effectDone) {
		return fail("external_capture_latency", nil)
	}
	baseline := records(w.Gateway)
	var agenda contracts.ReadResult
	agendaClient := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
	agendaErr := agendaClient.Call(ctx, "calendar.agenda", contracts.AgendaQuery{Start: "2026-11-01T00:00:00Z", End: "2026-11-04T12:00:00Z", CalendarID: calendar().ID}, &agenda)
	add("agenda_returns_occurrences", agendaErr == nil && len(agenda.Records) == count)

	switch s.ID {
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
	case "CAL-CRUD":
		current := records(w.Gateway)[0]
		if err = submit("calendar.update_event", map[string]any{"id": current.ID, "span": "this", "title": tag + " updated"}, current.Version); err != nil {
			return fail("update_event", err)
		}
		if !awaitCount(1) {
			return fail("updated_visible", nil)
		}
		add("update_preserves_bridge_id", records(w.Gateway)[0].ID == current.ID)
		current = records(w.Gateway)[0]
		var stale contracts.CommandRecord
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		staleErr := client.Call(ctx, "calendar.update_event", contracts.Command{Args: mustJSON(map[string]any{"id": current.ID, "span": "this", "title": tag + " stale"}), ExpectedVersion: current.Version - 1, WaitMS: 15000}, &stale)
		var conflict *contracts.Error
		add("stale_version_rejected", errors.As(staleErr, &conflict) && conflict.Code == "conflict" && conflict.Current != nil && conflict.Current.ID == current.ID)
		if err = submit("calendar.delete_event", map[string]any{"id": current.ID, "span": "this"}, current.Version); err != nil {
			return fail("delete_event", err)
		}
		add("delete_absence_visible", awaitCount(0))
		if err = submit("calendar.create_calendar", map[string]any{"title": "Bridge Test"}, 0); err != nil {
			return fail("create_calendar", err)
		}
		add("create_calendar_observed", last.CommitSeq > 0)
		if level != "sim" {
			var evidence struct {
				NativeKey string `json:"native_key"`
			}
			if json.Unmarshal(last.Evidence, &evidence) != nil || evidence.NativeKey == "" {
				return fail("new_calendar_identity", nil)
			}
			defer func() {
				if _, err := w.Truth("fixture", map[string]any{"source": "calendar", "action": "cleanup_calendar", "calendar_id": evidence.NativeKey, "tag": tag}); err != nil {
					result.Passed = false
					result.Assertions = append(result.Assertions, Assertion{Name: "created_calendar_cleanup", Passed: false})
				}
			}()
		}
	case "CAL-RECUR-THIS", "CAL-MOVED-OCCURRENCE":
		target := baseline[1]
		args := map[string]any{"id": target.ID, "span": "this", "title": tag + " exception"}
		if s.ID == "CAL-MOVED-OCCURRENCE" {
			aliases, _ := gatewayAliases(w)
			native := ""
			for alias, id := range aliases {
				prefix := "calendar/calendar.event/"
				if id == target.ID && strings.HasPrefix(alias, prefix) {
					native = strings.TrimPrefix(alias, prefix)
				}
			}
			_, err = fixture("move", native, map[string]any{"index": 1, "new_title": tag + " exception", "start": "2026-11-02T08:30:00Z"})
		} else {
			err = submit("calendar.update_event", args, target.Version)
		}
		if err != nil {
			return fail("update_occurrence", err)
		}
		if !awaitCount(3) {
			return fail("three_occurrences_after_exception", nil)
		}
		after := records(w.Gateway)
		kept, unchanged := false, 0
		for _, r := range after {
			if r.ID == target.ID {
				kept = raw(r)["occurrence_date"] == raw(target)["occurrence_date"] && raw(r)["is_exception"] == true
			}
			if r.ID != target.ID && recordTitle(r) == title {
				unchanged++
			}
		}
		add("original_occurrence_identity_preserved", kept)
		add("other_occurrences_unchanged", unchanged == 2)
	case "CAL-RECUR-FUTURE":
		target := baseline[1]
		if err = submit("calendar.update_event", map[string]any{"id": target.ID, "span": "future", "title": tag + " future"}, target.Version); err != nil {
			return fail("future_update", err)
		}
		if !awaitCount(3) {
			return fail("split_occurrences_visible", nil)
		}
		after := records(w.Gateway)
		oldCount, newCount := 0, 0
		series := map[string]bool{}
		for _, r := range after {
			if recordTitle(r) == title {
				oldCount++
			}
			if recordTitle(r) == tag+" future" {
				newCount++
			}
			series[fmt.Sprint(raw(r)["series_id"])] = true
		}
		add("future_span_splits_series", oldCount == 1 && newCount == 2 && len(series) == 2)
		target = after[1]
		if err = submit("calendar.delete_event", map[string]any{"id": target.ID, "span": "future"}, target.Version); err != nil {
			return fail("delete_future", err)
		}
		add("delete_future_preserves_past", awaitCount(1) && recordTitle(records(w.Gateway)[0]) == title)
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
					eviction = eviction || change.Change.Op == "evict"
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
				if r.Collection == "calendar.calendar" && raw(r)["writable"] == false {
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
		add("readonly_rejected_before_dispatch", errors.Is(err, contracts.ErrUnsupported) && last.Disposition == "rejected_before_dispatch")
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
	}
	if s.ID != "CAL-PERMISSION-LOSS" {
		if level == "sim" {
			add("independent_simtruth_convergence", w.Await(5*time.Second, func() bool { return nativeConvergence(w) }) == nil)
		} else {
			truth, truthErr := w.Truth("observe", map[string]any{"source": "calendar", "tag": tag})
			expected, _ := truth["records"].([]any)
			actual := records(w.Gateway)
			equal := truthErr == nil && len(expected) == len(actual)
			for _, item := range expected {
				d, ok := item.(map[string]any)
				if !ok {
					equal = false
					continue
				}
				found := false
				for _, r := range actual {
					v := raw(r)
					if d["title"] == v["title"] && sameCalendarTime(d["start"], v["start"]) && sameCalendarTime(d["end"], v["end"]) && d["all_day"] == v["all_day"] {
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
		add("latency_"+name, latency.P95 < 1000 && latency.Samples > 0)
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

func calendarHostConditions() map[string]any {
	load, _ := exec.Command("sysctl", "-n", "vm.loadavg").Output()
	fields := strings.Fields(strings.Trim(string(load), "{} \n"))
	slots := 0
	entries, _ := os.ReadDir("/tmp")
	for _, entry := range entries {
		if strings.HasPrefix(entry.Name(), "apple-bridge-heavy-") && strings.HasSuffix(entry.Name(), ".lock") {
			slots++
		}
	}
	return map[string]any{"load_average": fields, "heavy_slots_in_use": slots, "clock": "native CLOCK_UPTIME_RAW for real fixtures; lab monotonic fixture return for sim", "metadata_endpoint": "Apple effect commit to gateway/local SSE on each replica"}
}
