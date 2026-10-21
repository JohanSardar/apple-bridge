package demo

import (
	"fmt"
	"math/rand/v2"
	"time"
)

func (d *Dataset) people(rng *rand.Rand) {
	names := []string{"Aster", "Beryl", "Cobalt", "Dune", "Ember", "Fern", "Garnet", "Halo", "Indigo", "Juniper", "Kite", "Lumen"}
	for i := 0; i < 60; i++ {
		name := fmt.Sprintf("%s%02d", names[i%len(names)], i)
		data := map[string]any{"given_name": "Demo " + name, "family_name": "Example", "organization": "Synthetic Studio", "phones": []any{map[string]string{"label": "mobile", "value": fmt.Sprintf("+1 555 01%02d", i)}}, "emails": []any{map[string]string{"label": "home", "value": fmt.Sprintf("demo.person%02d@example.com", i)}}, "container_id": "demo-synthetic", "writable": true}
		if i%2 == 0 {
			data["image"] = map[string]any{"digest": d.blob(picture(rng.Uint64(), 96, 96, false)), "state": "present"}
		}
		d.put("contacts.contact", contactKey(i), data)
	}
	for i := 0; i < 60; i++ {
		service := []string{"iMessage", "SMS", "RCS"}[i%3]
		d.put("messages.handle", handleKey(i), map[string]any{"address": fmt.Sprintf("+1 555 01%02d", i), "service": service})
	}
	for c := 0; c < 40; c++ {
		members := 1
		if c == 0 || c >= 25 && c < 39 {
			members = 3 + (c % 10)
		}
		participants := []any{}
		for j := 0; j < members; j++ {
			participants = append(participants, reference("messages.handle", handleKey((c+j)%60)))
		}
		style := "direct"
		if members > 1 {
			style = "group"
		}
		name := ""
		if members > 1 && c%2 == 0 {
			name = fmt.Sprintf("Demo %s Club", names[c%len(names)])
		}
		if c == 0 {
			name = "Demo Studio"
		}
		if c == 1 {
			name = "Demo Busy Workshop"
		}
		if c == 39 {
			name = "Demo Performance Archive"
		}
		data := map[string]any{"display_name": name, "service": []string{"iMessage", "SMS", "RCS"}[c%3], "style": style, "participants": participants, "is_archived": c == 37 || c == 38, "capabilities": []string{}, "last_read_at": Epoch.Add(-48 * time.Hour)}
		d.put("messages.chat", chatKey(c), data)
	}
}
