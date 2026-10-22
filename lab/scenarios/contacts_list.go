package scenarios

import (
	"context"
	"encoding/json"
	"os/exec"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runContactsList(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "contacts:read,messages:read,system:read"})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	digest := strings.Repeat("b", 64)
	fixtures := []struct {
		collection, key string
		data            map[string]any
	}{
		{"contacts.contact", "con-list-alpha", map[string]any{"given_name": "Zoe", "family_name": "Adams", "is_me": true, "organization": "Bridge Test", "emails": []any{map[string]any{"label": "work", "value": "con-list@example.com"}}, "thumbnail_image": map[string]any{"digest": digest, "state": "present"}}},
		{"contacts.contact", "con-list-beta", map[string]any{"given_name": "Amy", "family_name": "Zulu", "organization": "Bridge Test"}},
		{"contacts.contact", "con-list-number", map[string]any{"organization": "123 Test", "contact_type": "organization"}},
		{"contacts.group", "con-list-group", map[string]any{"name": "Bridge Test List", "member_ids": []any{map[string]any{"$ref": map[string]any{"collection": "contacts.contact", "native_key": "con-list-alpha"}}}}},
		{"messages.handle", "con-list-handle", map[string]any{"address": "con-list@example.com", "service": "iMessage"}},
		{"messages.chat", "con-list-chat", map[string]any{"display_name": "Bridge Test Conversation", "participants": []any{map[string]any{"$ref": map[string]any{"collection": "messages.handle", "native_key": "con-list-handle"}}}}},
	}
	for _, f := range fixtures {
		if err = w.ControlAction("/external-change", map[string]any{"collection": f.collection, "key": f.key, "data": f.data}); err != nil {
			break
		}
	}
	add("fixture_capture", err == nil && w.FlushCaptures(10*time.Second) == nil)
	add("replica_converged", w.Await(10*time.Second, func() bool {
		head, err := w.Cursor(w.Gateway)
		if err != nil || !contactsListTruth(w) {
			return false
		}
		for _, replica := range w.Replicas {
			cursor, err := w.Cursor(replica)
			if err != nil || cursor != head {
				return false
			}
		}
		return true
	}) == nil)
	aliases, _ := gatewayAliases(w)
	alpha := aliases[aliasKey(contracts.Contacts, "contacts.contact", "con-list-alpha")]
	group := aliases[aliasKey(contracts.Contacts, "contacts.group", "con-list-group")]
	chat := aliases[aliasKey(contracts.Messages, "messages.chat", "con-list-chat")]
	add("fixture_ids", alpha != "" && group != "" && chat != "")
	for index, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		for _, protocol := range []string{"rest", "mcp"} {
			client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
			prefix := protocol + "_" + string(rune('0'+index))
			var out struct {
				Sections []struct {
					Key      string
					Contacts []struct {
						Contact         contracts.Record
						DisplayName     string `json:"display_name"`
						Avatar          *contracts.HandleAvatar
						IsMe            *bool    `json:"is_me"`
						ConversationIDs []string `json:"conversation_ids"`
					}
				}
				Groups []struct {
					Group contracts.Record
					Count int
				}
				Cursor contracts.Cursor
			}
			err = client.Call(ctx, "contacts.list", map[string]any{"group": group, "sort": "family", "display_order": "given"}, &out)
			add(prefix+"_available", err == nil)
			if err != nil {
				continue
			}
			valid := len(out.Sections) == 1 && out.Sections[0].Key == "A" && len(out.Sections[0].Contacts) == 1
			add(prefix+"_membership_sections", valid)
			if valid {
				c := out.Sections[0].Contacts[0]
				add(prefix+"_name_identity_photo", c.Contact.ID == alpha && c.DisplayName == "Zoe Adams" && c.IsMe != nil && *c.IsMe && c.Avatar != nil && c.Avatar.Digest == digest)
				add(prefix+"_linked_conversation", len(c.ConversationIDs) == 1 && c.ConversationIDs[0] == chat)
			}
			count := false
			for _, g := range out.Groups {
				if g.Group.ID == group {
					count = g.Count == 1
				}
			}
			add(prefix+"_group_count", count)
			err = client.Call(ctx, "contacts.list", map[string]any{"sort": "given", "display_order": "family"}, &out)
			found := false
			for _, section := range out.Sections {
				for _, c := range section.Contacts {
					if c.Contact.ID == alpha {
						found = section.Key == "Z" && c.DisplayName == "Adams Zoe"
					}
				}
			}
			add(prefix+"_device_preferences", err == nil && found)
			add(prefix+"_invalid_sort", client.Call(ctx, "contacts.list", map[string]any{"sort": "unsupported"}, nil) != nil)
			add(prefix+"_unknown_group", client.Call(ctx, "contacts.list", map[string]any{"group": "missing"}, nil) != nil)
			restricted := endpoint
			restricted.Token, _ = w.ScopedToken(endpoint, "contacts:read")
			err = (scripted.Client{Endpoint: restricted, Protocol: protocol}).Call(ctx, "contacts.list", map[string]any{"group": group}, &out)
			private := err == nil
			for _, section := range out.Sections {
				for _, c := range section.Contacts {
					private = private && len(c.ConversationIDs) == 0
				}
			}
			add(prefix+"_conversation_scope", private)
			restricted.Token, _ = w.ScopedToken(endpoint, "system:read")
			var raw map[string]json.RawMessage
			err = (scripted.Client{Endpoint: restricted, Protocol: protocol}).Call(ctx, "contacts.list", map[string]any{}, &raw)
			add(prefix+"_contact_scope", err == nil && string(raw["sections"]) == "[]" && string(raw["groups"]) == "[]")
		}
	}
	add("deleted_contact_fixture", w.ControlAction("/external-change", map[string]any{"collection": "contacts.contact", "key": "con-list-alpha", "delete": true}) == nil && w.FlushCaptures(10*time.Second) == nil)
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		add("deleted_members_do_not_count", w.Await(10*time.Second, func() bool {
			var out contracts.ContactsListResult
			err := (scripted.Client{Endpoint: endpoint}).Call(ctx, "contacts.list", contracts.ContactsListQuery{Group: group}, &out)
			if err != nil || out.Count != 0 || len(out.Sections) != 0 {
				return false
			}
			for _, g := range out.Groups {
				if g.Group.ID == group {
					return g.Count == 0
				}
			}
			return false
		}) == nil)
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}

func contactsListTruth(w *world.World) bool {
	observations, err := w.Inspect()
	if err != nil {
		return false
	}
	filtered := []contracts.Observation{}
	for _, observation := range observations {
		source := contracts.CollectionSource(observation.Collection)
		if source == contracts.Contacts || source == contracts.Messages {
			filtered = append(filtered, observation)
		}
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return false
	}
	expected, err := truthRecords(filtered, aliases)
	if err != nil {
		return false
	}
	actual, err := w.Records(w.Gateway)
	if err != nil {
		return false
	}
	for i, record := range actual {
		if record.Collection == "messages.handle" {
			var fields map[string]any
			if json.Unmarshal(record.Data, &fields) != nil {
				return false
			}
			delete(fields, "contact_id")
			actual[i].Data, _ = json.Marshal(fields)
		}
	}
	return reflect.DeepEqual(recordMultiset(withoutCommands(actual), false), recordMultiset(expected, false))
}
