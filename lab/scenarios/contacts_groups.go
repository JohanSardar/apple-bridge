package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runContactGroups(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 0, 0, world.BlobOptions{Scope: "contacts:read,contacts:write,system:read"})
	add("world_start", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	for _, protocol := range []string{"rest", "mcp"} {
		client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
		submit := func(op string, args map[string]any, version uint64) contracts.CommandRecord {
			var result contracts.CommandRecord
			err := client.Call(ctx, op, map[string]any{"command_id": contracts.NewCommandID(), "args": args, "expected_version": version, "wait_ms": 15000}, &result)
			add(protocol+"_"+op, err == nil && result.State == "observed" && len(result.ObservedRecordIDs) == 1)
			return result
		}
		read := func(collection, id string) contracts.Record {
			var result contracts.ReadResult
			err := client.Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, ID: id}, &result)
			if err != nil || len(result.Records) != 1 {
				add(protocol+"_read_"+collection, false)
				return contracts.Record{}
			}
			return result.Records[0]
		}
		created := submit("contacts.create", map[string]any{"given_name": "[ab-test groups]", "organization": "Bridge Test"}, 0)
		if len(created.ObservedRecordIDs) != 1 {
			continue
		}
		contactID := created.ObservedRecordIDs[0]
		created = submit("contacts.create_group", map[string]any{"name": "Bridge Test [ab-test groups]"}, 0)
		if len(created.ObservedRecordIDs) != 1 {
			continue
		}
		groupID := created.ObservedRecordIDs[0]
		group := read("contacts.group", groupID)
		submit("contacts.rename_group", map[string]any{"id": groupID, "name": "Bridge Test [ab-test groups] renamed"}, group.Version)
		var stale contracts.CommandRecord
		staleErr := client.Call(ctx, "contacts.delete_group", map[string]any{"command_id": contracts.NewCommandID(), "args": map[string]any{"id": groupID}, "expected_version": group.Version, "wait_ms": 15000}, &stale)
		add(protocol+"_stale_group_rejected", staleErr != nil || stale.State == "rejected")
		group = read("contacts.group", groupID)
		submit("contacts.add_member", map[string]any{"id": groupID, "contact_id": contactID}, group.Version)
		var list contracts.ContactsListResult
		err := client.Call(ctx, "contacts.list", contracts.ContactsListQuery{Group: groupID}, &list)
		add(protocol+"_membership_read_model", err == nil && list.Count == 1 && len(list.Sections) == 1 && list.Sections[0].Contacts[0].Contact.ID == contactID)
		group = read("contacts.group", groupID)
		submit("contacts.remove_member", map[string]any{"id": groupID, "contact_id": contactID}, group.Version)
		err = client.Call(ctx, "contacts.list", contracts.ContactsListQuery{Group: groupID}, &list)
		add(protocol+"_membership_removed", err == nil && list.Count == 0)
		group = read("contacts.group", groupID)
		submit("contacts.add_member", map[string]any{"id": groupID, "contact_id": contactID}, group.Version)
		group = read("contacts.group", groupID)
		submit("contacts.delete_group", map[string]any{"id": groupID}, group.Version)
		contact := read("contacts.contact", contactID)
		add(protocol+"_group_delete_preserves_contact", contact.ID == contactID)
		submit("contacts.delete", map[string]any{"id": contactID}, contact.Version)
		var raw map[string]json.RawMessage
		add(protocol+"_deleted_group_not_found", client.Call(ctx, "contacts.list", contracts.ContactsListQuery{Group: groupID}, &raw) != nil)
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
