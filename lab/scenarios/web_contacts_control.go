package scenarios

import (
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webContactsRead(w *world.World, update bool) (any, error) {
	records, err := w.Records(w.Gateway)
	if err != nil {
		return nil, err
	}
	var image any
	for _, record := range records {
		if record.Collection == "contacts.contact" {
			var data map[string]any
			json.Unmarshal(record.Data, &data)
			if data["image"] != nil {
				image = data["image"]
				break
			}
		}
	}
	name := "Zoe"
	if update {
		name = "Zoe Updated"
	}
	rich := map[string]any{"container_id": "web-con-container", "given_name": name, "family_name": "Adler", "middle_name": "M", "organization": "Bridge Test Studio", "job_title": "Designer", "phonetic_given_name": "Zo-ee", "is_me": true, "writable": true, "image": image, "phones": []any{map[string]any{"label": "_$!<Mobile>!$_", "value": "+15555550123"}}, "emails": []any{map[string]any{"label": "work", "value": "contacts-read@example.com"}}, "urls": []any{map[string]any{"label": "home", "value": "https://example.com/profile"}, map[string]any{"label": "unsafe", "value": "javascript:alert(1)"}}, "postal_addresses": []any{map[string]any{"label": "work", "value": map[string]any{"street": "1 Example Street", "city": "Test City", "country": "Test Country"}}}, "birthday": map[string]any{"year": 1990, "month": 5, "day": 20}, "dates": []any{map[string]any{"label": "anniversary", "value": map[string]any{"month": 6, "day": 10}}}, "relations": []any{map[string]any{"label": "friend", "value": "Example Friend"}}, "social_profiles": []any{map[string]any{"label": "work", "value": map[string]any{"username": "example", "service": "Example", "url": "https://example.com/social"}}}, "instant_messages": []any{map[string]any{"label": "work", "value": map[string]any{"username": "example", "service": "Test IM"}}}, "note": "hidden sensitive note"}
	fixtures := []struct {
		collection, key string
		data            map[string]any
	}{
		{"contacts.contact", "web-con-rich", rich},
		{"contacts.contact", "web-con-other", map[string]any{"container_id": "web-con-container", "given_name": "Amy", "family_name": "Zulu", "organization": "Bridge Test Studio", "writable": true}},
		{"contacts.contact", "web-con-number", map[string]any{"container_id": "web-con-container", "contact_type": "organization", "organization": "123 Test Studio"}},
		{"contacts.container", "web-con-container", map[string]any{"name": "Bridge Test Account", "identifier": "web-con-container"}},
		{"contacts.group", "web-con-group", map[string]any{"name": "Bridge Test Friends", "member_ids": []any{rmRef("contacts.contact", "web-con-rich"), rmRef("contacts.contact", "web-con-other"), rmRef("contacts.contact", "web-con-number")}}},
		{"messages.handle", "web-con-handle", map[string]any{"address": "contacts-read@example.com", "service": "iMessage"}},
		{"messages.chat", "web-con-chat", map[string]any{"display_name": "Bridge Test Recent", "participants": []any{rmRef("messages.handle", "web-con-handle")}, "service": "iMessage", "style": "direct"}},
	}
	if update {
		fixtures = fixtures[:1]
	}
	for _, fixture := range fixtures {
		if err = w.ControlAction("/external-change", map[string]any{"collection": fixture.collection, "key": fixture.key, "data": fixture.data}); err != nil {
			return nil, err
		}
	}
	if err = w.FlushCaptures(10 * time.Second); err != nil {
		return nil, err
	}
	aliases, err := gatewayAliases(w)
	return map[string]any{"contact": aliases[aliasKey(contracts.Contacts, "contacts.contact", "web-con-rich")], "group": aliases[aliasKey(contracts.Contacts, "contacts.group", "web-con-group")], "chat": aliases[aliasKey(contracts.Messages, "messages.chat", "web-con-chat")]}, err
}
