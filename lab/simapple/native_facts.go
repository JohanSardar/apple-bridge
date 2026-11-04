package simapple

import (
	"encoding/json"
	"fmt"
	"net"
	"sort"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

const nativeFactsFrameLimit = 262144

type nativeSnapshot struct {
	operation  string
	items      []json.RawMessage
	scope      any
	generation string
	expires    time.Time
}

type factSession struct {
	fence     int
	counter   int
	snapshots map[string]nativeSnapshot
}

func (s *factSession) hello(frame map[string]json.RawMessage) {
	var fence int
	json.Unmarshal(frame["owner_fence"], &fence)
	s.fence = fence
}

func (f *fakeApple) nativeFactsPage(conn net.Conn, s *factSession, frame map[string]json.RawMessage) error {
	var id, op string
	var args map[string]json.RawMessage
	json.Unmarshal(frame["id"], &id)
	json.Unmarshal(frame["op"], &op)
	json.Unmarshal(frame["args"], &args)
	fail := func(code string) error {
		return writeNative(conn, map[string]any{"t": "Error", "id": id, "code": code})
	}
	limit := 100
	if raw, ok := frame["limit"]; ok && string(raw) != "null" {
		if json.Unmarshal(raw, &limit) != nil {
			return fail("invalid")
		}
	}
	if id == "" || len(args) != 0 || limit < 1 || limit > 1000 || s.fence < 1 {
		return fail("invalid")
	}
	if s.snapshots == nil {
		s.snapshots = map[string]nativeSnapshot{}
	}
	for token, snapshot := range s.snapshots {
		if time.Now().After(snapshot.expires) {
			delete(s.snapshots, token)
		}
	}
	var token string
	offset := 0
	if string(frame["snapshot"]) == "null" || len(frame["snapshot"]) == 0 {
		facts, code := f.nativeFacts(op, s.fence)
		if code != "" {
			return fail(code)
		}
		s.counter++
		token = "sim-snapshot-" + strconv.Itoa(s.counter)
		facts.expires = time.Now().Add(time.Minute)
		s.snapshots[token] = facts
	} else {
		var cursor string
		json.Unmarshal(frame["snapshot"], &token)
		json.Unmarshal(frame["cursor"], &cursor)
		stored, ok := s.snapshots[token]
		if !ok {
			return fail("expired")
		}
		position, err := strconv.Atoi(strings.TrimPrefix(cursor, token+":"))
		if stored.operation != op || !strings.HasPrefix(cursor, token+":") || err != nil || position < 0 || position > len(stored.items) {
			return fail("invalid")
		}
		offset = position
	}
	snapshot := s.snapshots[token]
	end := offset + limit
	if end > len(snapshot.items) {
		end = len(snapshot.items)
	}
	var page map[string]any
	for {
		terminal := end == len(snapshot.items)
		page = map[string]any{"t": "FactPage", "id": id, "op": op, "snapshot": token, "items": snapshot.items[offset:end], "cursor": nil, "complete": terminal, "scope": snapshot.scope, "generation": snapshot.generation}
		if !terminal {
			page["cursor"] = token + ":" + strconv.Itoa(end)
		}
		body, _ := json.Marshal(page)
		if len(body) <= nativeFactsFrameLimit || end <= offset+1 {
			break
		}
		end = offset + (end-offset)/2
	}
	if end == len(snapshot.items) {
		delete(s.snapshots, token)
	}
	return writeNative(conn, page)
}

func (f *fakeApple) nativeFacts(op string, fence int) (nativeSnapshot, string) {
	var capture contracts.Capture
	var collection string
	switch op {
	case "agenda.calendars", "agenda.events":
		f.mu.Lock()
		denied := f.calendarDenied
		if !denied {
			capture = f.calendarScanLocked()
			capture.Generation = f.generation
		}
		f.mu.Unlock()
		if denied {
			return nativeSnapshot{}, "permission_missing"
		}
		collection = map[string]string{"agenda.calendars": "calendar.calendar", "agenda.events": "calendar.event"}[op]
	case "contacts.containers", "contacts.groups", "contacts.contacts":
		f.mu.Lock()
		denied := f.contactPermissionMissing
		f.mu.Unlock()
		if denied {
			return nativeSnapshot{}, "permission_missing"
		}
		capture = f.contactSnapshot(false)
		collection = map[string]string{"contacts.containers": "contacts.container", "contacts.groups": "contacts.group", "contacts.contacts": "contacts.contact"}[op]
	default:
		return nativeSnapshot{}, "unsupported"
	}
	items := []json.RawMessage{}
	for _, observation := range capture.Observations {
		if observation.Collection != collection {
			continue
		}
		raw, err := json.Marshal(observation)
		if err != nil {
			return nativeSnapshot{}, "unavailable"
		}
		items = append(items, raw)
	}
	if op == "contacts.contacts" {
		digests := make([]string, 0, len(capture.BlobHandles))
		for digest := range capture.BlobHandles {
			digests = append(digests, digest)
		}
		sort.Strings(digests)
		for _, digest := range digests {
			raw, _ := json.Marshal(map[string]string{"kind": "blob_handle", "digest": digest, "handle": fmt.Sprintf("%d:%s", fence, digest)})
			items = append(items, raw)
		}
	}
	return nativeSnapshot{operation: op, items: items, scope: map[string]any{"collections": []string{collection}, "history_token": ""}, generation: capture.Generation}, ""
}
