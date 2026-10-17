package simapple

import (
	"encoding/json"
	"net/http"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/applepeer"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func calendarData(o contracts.Observation) map[string]any {
	var d map[string]any
	decodeNative(o.Data, &d)
	return d
}
func calendarObservation(key, collection string, data map[string]any) contracts.Observation {
	raw, _ := json.Marshal(data)
	return contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, Data: raw, ObservedAt: time.Now().UTC()}
}
func nativeReference(collection, key string) any {
	return map[string]any{"$ref": map[string]string{"collection": collection, "native_key": key}}
}
func calendarCapture(records []contracts.Observation) contracts.Capture {
	return contracts.Capture{Source: contracts.Calendar, Observations: records, ScanID: contracts.NewID("scan"), Scope: []string{"calendar.calendar", "calendar.event"}, ScanStartedAt: time.Now().UTC(), CompletionEvidence: "complete"}
}
func (f *fakeApple) calendarNow() time.Time {
	if f.calendarClock.IsZero() {
		return time.Now()
	}
	return f.calendarClock.In(time.Local)
}
func (f *fakeApple) calendarInWindow(o contracts.Observation) bool {
	if o.Collection != "calendar.event" || o.Kind == "absent" {
		return true
	}
	d := calendarData(o)
	start, _ := d["start"].(string)
	end, _ := d["end"].(string)
	now := f.calendarNow()
	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, now.Location())
	left, right := today.AddDate(0, 0, -730), today.AddDate(0, 0, 730)
	if len(start) == 10 {
		return start < right.Format(time.DateOnly) && end > left.Format(time.DateOnly)
	}
	a, e := time.Parse(time.RFC3339Nano, start)
	b, g := time.Parse(time.RFC3339Nano, end)
	return e == nil && g == nil && a.Before(right) && b.After(left)
}
func (f *fakeApple) calendarScanLocked() contracts.Capture {
	if f.calendarPublished == nil {
		f.calendarPublished = map[string]contracts.Observation{}
	}
	observations := []contracts.Observation{}
	current := map[string]contracts.Observation{}
	for key, o := range f.records {
		if contracts.CollectionSource(o.Collection) != contracts.Calendar {
			continue
		}
		if f.calendarInWindow(o) {
			current[key] = o
			observations = append(observations, o)
		}
	}
	for key, o := range f.calendarPublished {
		if _, ok := current[key]; !ok {
			if old, exists := f.records[key]; exists && old.Kind != "absent" && !f.calendarInWindow(old) {
				o.Kind = "evict"
				o.Revalidated = true
			} else {
				o.Kind = "absent"
				o.Revalidated = true
			}
			o.ObservedAt = time.Now().UTC()
			observations = append(observations, o)
		}
	}
	f.calendarPublished = current
	return calendarCapture(observations)
}
func (f *fakeApple) calendarControl(mux *http.ServeMux) {
	mux.HandleFunc("POST /calendar", func(w http.ResponseWriter, r *http.Request) {
		var request struct {
			Action string         `json:"action"`
			Key    string         `json:"key"`
			Data   map[string]any `json:"data"`
			Now    string         `json:"now"`
		}
		if json.NewDecoder(r.Body).Decode(&request) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		f.mu.Lock()
		switch request.Action {
		case "calendar":
			d := request.Data
			if d == nil {
				d = map[string]any{"title": "Bridge Test", "writable": true, "account": "test", "type": "local", "color": "#00aaff"}
			}
			f.records[request.Key] = calendarObservation(request.Key, "calendar.calendar", d)
		case "create":
			d := request.Data
			d["calendar_id"] = nativeReference("calendar.calendar", "test-calendar")
			if d["all_day"] == nil {
				d["all_day"] = false
			}
			if d["time_zone"] == nil {
				d["time_zone"] = "America/New_York"
			}
			d["is_exception"] = false
			d["attendees"] = []any{}
			d["alarms"] = []any{}
			d["availability"] = 0
			d["status"] = 1
			base := calendarObservation(request.Key, "calendar.event", d)
			expanded, err := expandCalendar(base)
			if err != nil {
				f.mu.Unlock()
				http.Error(w, "invalid", 400)
				return
			}
			for _, o := range expanded {
				f.records[o.NativeKey] = o
			}

		case "move":
			o, ok := f.records[request.Key]
			if !ok || o.Collection != "calendar.event" || o.Kind == "absent" {
				f.mu.Unlock()
				http.Error(w, "not found", 404)
				return
			}
			value := calendarData(o)
			start, _ := request.Data["start"].(string)
			a, e := time.Parse(time.RFC3339Nano, start)
			old, _ := value["start"].(string)
			oldEnd, _ := value["end"].(string)
			b, g := time.Parse(time.RFC3339Nano, old)
			end, h := time.Parse(time.RFC3339Nano, oldEnd)
			if e != nil || g != nil || h != nil {
				f.mu.Unlock()
				http.Error(w, "invalid", 400)
				return
			}
			value["start"] = a.UTC().Format(time.RFC3339Nano)
			value["end"] = a.Add(end.Sub(b)).UTC().Format(time.RFC3339Nano)
			value["is_exception"] = true
			if title, ok := request.Data["new_title"].(string); ok {
				value["title"] = title
			}
			o.Data, _ = json.Marshal(value)
			o.ObservedAt = time.Now().UTC()
			f.records[request.Key] = o
		case "churn":
			for key, o := range f.records {
				if o.Collection == "calendar.event" && o.Kind != "absent" {
					delete(f.records, key)
					o.NativeKey = "churn/" + key
					f.records[o.NativeKey] = o
					delete(f.calendarPublished, key)
				}
			}
		case "clock":
			now, err := time.Parse(time.RFC3339Nano, request.Now)
			if err != nil {
				f.mu.Unlock()
				http.Error(w, "invalid", 400)
				return
			}
			f.calendarClock = now
		case "delete":
			o, exists := f.records[request.Key]
			if !exists {
				f.mu.Unlock()
				http.Error(w, "not found", 404)
				return
			}
			o.Kind = "absent"
			o.Revalidated = true
			o.ObservedAt = time.Now().UTC()
			f.records[request.Key] = o
		default:
			f.mu.Unlock()
			http.Error(w, "invalid", 400)
			return
		}
		c := f.calendarScanLocked()
		denied := f.calendarDenied
		err := f.saveLocked()
		f.mu.Unlock()
		if err != nil {
			http.Error(w, "persist", 500)
			return
		}
		if !denied {
			if f.enqueue(r.Context(), c, true) != nil {
				return
			}
		}
		json.NewEncoder(w).Encode(map[string]any{"effect_done": true})
	})
}
func (f *fakeApple) isCalendarFrame(frame contracts.IPCFrame) bool {
	if frame.Action != nil && strings.HasPrefix(frame.Action.Op, "calendar.") {
		return true
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if t, ok := f.tickets[frame.Ticket]; ok && strings.HasPrefix(t.Action.Op, "calendar.") {
		return true
	}
	return false
}
func (f *fakeApple) calendarCommand(wire *applepeer.Wire, frame contracts.IPCFrame, fence uint64, emit func(contracts.Capture) error, checkpoint func(contracts.Source) string) error {
	conflictKey := ""
	if frame.Action != nil {
		conflictKey = frame.Action.NativeKey
	}
	reject := func(problem *contracts.Error) error {
		rejected := contracts.IPCFrame{T: "Rejected", CommandID: frame.CommandID, Disposition: "rejected_before_dispatch", Error: problem}
		if problem.Code == "conflict" && conflictKey != "" {
			f.mu.Lock()
			o, ok := f.records[conflictKey]
			f.mu.Unlock()
			if ok {
				o.ObservedAt = time.Now().UTC()
				if err := emit(calendarCapture([]contracts.Observation{o})); err != nil {
					return err
				}
				rejected.Evidence, _ = json.Marshal(contracts.ConflictObservation{NativeKey: conflictKey, CaptureCheckpoint: checkpoint(contracts.Calendar)})
			}
		}
		_, err := wire.Send(rejected)
		return err
	}
	f.mu.Lock()
	if f.calendarDenied {
		f.mu.Unlock()
		return reject(contracts.ErrUnavailable)
	}
	if frame.T == "Prepare" {
		action := *frame.Action
		if action.CalendarArgs == nil || action.CalendarArgs.Span == "all" {
			f.mu.Unlock()
			return reject(contracts.ErrUnsupported)
		}
		var before json.RawMessage
		if action.NativeKey != "" {
			o, ok := f.records[action.NativeKey]
			if !ok || o.Kind == "absent" {
				f.mu.Unlock()
				return reject(contracts.ErrNotFound)
			}
			before = o.Data
			expected, native := map[string]any{}, calendarData(o)
			decodeNative(frame.ExpectedState, &expected)
			membership, _ := native["calendar_id"].(map[string]any)
			reference, _ := membership["$ref"].(map[string]any)
			if reference["native_key"] != action.ExpectedListKey {
				f.mu.Unlock()
				return reject(contracts.ErrConflict)
			}
			delete(expected, "calendar_id")
			delete(native, "calendar_id")
			if !reflect.DeepEqual(expected, native) {
				f.mu.Unlock()
				return reject(contracts.ErrConflict)
			}
		}
		for _, key := range []string{action.ListKey, action.ExpectedListKey} {
			if key != "" && !calendarWritable(f.records[key]) {
				f.mu.Unlock()
				return reject(contracts.ErrUnsupported)
			}
		}
		key := contracts.NewID("ticket")
		expires := time.Now().Add(30 * time.Second)
		if frame.ExpiresAt.Before(expires) {
			expires = frame.ExpiresAt
		}
		f.tickets[key] = ticket{CommandID: frame.CommandID, Principal: frame.Principal, Hash: frame.RequestHash, Action: action, Before: before, Expires: expires, Fence: fence}
		f.mu.Unlock()
		_, err := wire.Send(contracts.IPCFrame{T: "Prepared", CommandID: frame.CommandID, Ticket: key, NativeState: before, Route: "sim-calendar", Fingerprint: frame.RequestHash})
		return err
	}
	t, ok := f.tickets[frame.Ticket]
	conflictKey = t.Action.NativeKey
	delete(f.tickets, frame.Ticket)
	if !ok || t.CommandID != frame.CommandID || t.Fence != fence || frame.OwnerFence != fence || !time.Now().Before(t.Expires) {
		f.mu.Unlock()
		return reject(contracts.ErrExpired)
	}
	if t.Action.NativeKey != "" && string(f.records[t.Action.NativeKey].Data) != string(t.Before) {
		f.mu.Unlock()
		return reject(contracts.ErrConflict)
	}
	for _, key := range []string{t.Action.ListKey, t.Action.ExpectedListKey} {
		if key != "" && !calendarWritable(f.records[key]) {
			f.mu.Unlock()
			return reject(contracts.ErrUnsupported)
		}
	}
	if err := f.guard(guardEntry{CommandID: t.CommandID, Ticket: frame.Ticket, StartedAt: time.Now().UTC()}); err != nil {
		f.mu.Unlock()
		return reject(contracts.ErrUnavailable)
	}
	a := *t.Action.CalendarArgs
	native := t.Action.NativeKey
	if native == "" {
		native = contracts.NewID("event")
	}
	collection := "calendar.event"
	if t.Action.Op == "calendar.create_calendar" {
		collection = "calendar.calendar"
	}
	patchBytes, _ := json.Marshal(a)
	patch := map[string]any{}
	decodeNative(patchBytes, &patch)
	for _, k := range []string{"id", "calendar_id", "span"} {
		delete(patch, k)
	}
	target := f.records[native]
	data := calendarData(target)
	if data == nil {
		data = map[string]any{}
	}
	series, _ := data["series_id"].(string)
	original, _ := data["occurrence_date"].(string)
	targets := []string{native}
	if a.Span == "future" && series != "" {
		targets = nil
		for key, o := range f.records {
			d := calendarData(o)
			occ, _ := d["occurrence_date"].(string)
			if o.Kind != "absent" && d["series_id"] == series && occ >= original {
				targets = append(targets, key)
			}
		}
	}
	if a.Span == "future" && series != "" {
		prior := []string{}
		for key, o := range f.records {
			d := calendarData(o)
			occ, _ := d["occurrence_date"].(string)
			if o.Kind != "absent" && d["series_id"] == series && occ < original {
				prior = append(prior, key)
			}
		}
		for _, key := range prior {
			o := f.records[key]
			value := calendarData(o)
			value["recurrence"] = boundedCalendarRecurrence(value["recurrence"], len(prior))
			o.Data, _ = json.Marshal(value)
			o.ObservedAt = time.Now().UTC()
			f.records[key] = o
		}
	}
	newSeries := contracts.NewID("series")
	for _, key := range targets {
		o := f.records[key]
		value := calendarData(o)
		if value == nil {
			value = map[string]any{}
		}
		for k, v := range patch {
			if (k == "start" || k == "end") && key != native && a.Span == "future" {
				old, _ := value[k].(string)
				anchor, _ := data[k].(string)
				replacement, _ := v.(string)
				current, e := time.Parse(time.RFC3339Nano, old)
				before, b := time.Parse(time.RFC3339Nano, anchor)
				after, c := time.Parse(time.RFC3339Nano, replacement)
				if e == nil && b == nil && c == nil {
					value[k] = current.Add(after.Sub(before)).UTC().Format(time.RFC3339Nano)
					continue
				}
			}
			value[k] = v
		}
		if collection == "calendar.calendar" {
			value["writable"] = true
			value["account"] = "test"
			value["type"] = "local"
			value["color"] = "#00aaff"
		} else {
			value["calendar_id"] = nativeReference("calendar.calendar", t.Action.ListKey)
			if series != "" {
				if a.Span == "this" {
					value["is_exception"] = true
				} else {
					value["series_id"] = newSeries
					value["recurrence"] = boundedCalendarRecurrence(value["recurrence"], len(targets))
				}
			}
			if value["all_day"] == nil {
				value["all_day"] = false
			}
			if value["time_zone"] == nil {
				value["time_zone"] = "America/New_York"
			}
			if value["is_exception"] == nil {
				value["is_exception"] = false
			}
			if value["attendees"] == nil {
				value["attendees"] = []any{}
			}
			if value["alarms"] == nil {
				value["alarms"] = []any{}
			}
			if value["availability"] == nil {
				value["availability"] = 0
			}
			if value["status"] == nil {
				value["status"] = 1
			}
		}
		next := calendarObservation(key, collection, value)
		next.ScopedAlias = o.ScopedAlias
		if strings.HasSuffix(t.Action.Op, "delete_event") {
			next.Kind = "absent"
			next.Revalidated = true
		}
		if t.Action.Op == "calendar.create_event" && a.Recurrence != nil && len(*a.Recurrence) > 0 {
			expanded, expandErr := expandCalendar(next)
			if expandErr != nil {
				f.mu.Unlock()
				return expandErr
			}
			for _, o := range expanded {
				f.records[o.NativeKey] = o
			}
			native = expanded[0].NativeKey
		} else {
			f.records[key] = next
		}
	}
	f.effects[t.CommandID]++
	capture := f.calendarScanLocked()
	primary := f.records[native]
	err := f.saveLocked()
	f.mu.Unlock()
	if err != nil {
		return err
	}
	if err = emit(capture); err != nil {
		return err
	}
	if len(capture.Observations) > 64 {
		if err = emit(calendarCapture([]contracts.Observation{primary})); err != nil {
			return err
		}
	}
	evidence, _ := json.Marshal(map[string]any{"native_key": native, "fingerprint": t.Hash, "observed": true, "capture_checkpoint": checkpoint(contracts.Calendar), "absent": strings.HasSuffix(t.Action.Op, "delete_event")})
	outcome := contracts.IPCFrame{T: "Outcome", CommandID: t.CommandID, Disposition: "locally_observed", Evidence: evidence}
	f.mu.Lock()
	f.evidence[t.CommandID] = outcome
	err = f.saveLocked()
	f.mu.Unlock()
	if err != nil {
		return err
	}
	if err = f.guard(guardEntry{CommandID: t.CommandID, Outcome: &outcome}); err != nil {
		return err
	}
	_, err = wire.Send(outcome)
	return err
}

func expandCalendar(base contracts.Observation) ([]contracts.Observation, error) {
	d := calendarData(base)
	rules, ok := d["recurrence"].([]any)
	if !ok || len(rules) == 0 {
		base.ScopedAlias = "test/test-calendar/" + base.NativeKey
		return []contracts.Observation{base}, nil
	}
	ruleBytes, _ := json.Marshal(rules[0])
	var rule contracts.CalendarRecurrence
	if json.Unmarshal(ruleBytes, &rule) != nil || rule.Interval < 1 {
		return nil, contracts.ErrInvalid
	}
	zoneName, _ := d["time_zone"].(string)
	zone, err := time.LoadLocation(zoneName)
	if err != nil {
		zone = time.UTC
	}
	allDay, _ := d["all_day"].(bool)
	layout := time.RFC3339Nano
	if allDay {
		layout = time.DateOnly
	}
	start, _ := d["start"].(string)
	end, _ := d["end"].(string)
	a, e := time.ParseInLocation(layout, start, zone)
	b, f := time.ParseInLocation(layout, end, zone)
	if e != nil || f != nil || !b.After(a) {
		return nil, contracts.ErrInvalid
	}
	a = a.In(zone)
	b = b.In(zone)
	count := rule.Count
	if count == 0 {
		count = 1462
	}
	if count > 10000 {
		return nil, contracts.ErrCapacity
	}
	until, _ := time.Parse(time.RFC3339Nano, rule.Until)
	result := []contracts.Observation{}
	for i := 0; i < count; i++ {
		years, months, days := 0, 0, 0
		switch rule.Frequency {
		case "daily":
			days = i * rule.Interval
		case "weekly":
			days = i * 7 * rule.Interval
		case "monthly":
			months = i * rule.Interval
		case "yearly":
			years = i * rule.Interval
		default:
			return nil, contracts.ErrInvalid
		}
		occurrence := a.AddDate(years, months, days)
		finish := b.AddDate(years, months, days)
		if !until.IsZero() && occurrence.After(until) {
			break
		}
		value := map[string]any{}
		for k, v := range d {
			value[k] = v
		}
		original := occurrence.UTC().Format(time.RFC3339Nano)
		if allDay {
			original = occurrence.Format(time.DateOnly)
		}
		value["start"] = occurrence.UTC().Format(layout)
		value["end"] = finish.UTC().Format(layout)
		if allDay {
			value["start"] = occurrence.Format(time.DateOnly)
			value["end"] = finish.Format(time.DateOnly)
		}
		value["series_id"] = base.NativeKey
		value["occurrence_date"] = original
		key := base.NativeKey + "|" + original
		next := calendarObservation(key, "calendar.event", value)
		next.ScopedAlias = "test/test-calendar/" + key
		result = append(result, next)
	}
	if len(result) == 0 {
		return nil, contracts.ErrInvalid
	}
	return result, nil
}

func calendarWritable(o contracts.Observation) bool {
	data := calendarData(o)
	return o.Collection == "calendar.calendar" && o.Kind != "absent" && data["writable"] == true && data["type"] != "subscribed"
}

func boundedCalendarRecurrence(value any, count int) any {
	rules, ok := value.([]any)
	if !ok {
		return value
	}
	result := []any{}
	for _, raw := range rules {
		rule, ok := raw.(map[string]any)
		if !ok {
			return value
		}
		copy := map[string]any{}
		for k, v := range rule {
			copy[k] = v
		}
		copy["count"] = count
		delete(copy, "end")
		result = append(result, copy)
	}
	return result
}
