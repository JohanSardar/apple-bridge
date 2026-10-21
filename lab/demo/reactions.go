package demo

import (
	"fmt"
	"time"
)

func (d *Dataset) reactions() {
	for i, kind := range []string{"love", "like", "dislike", "laugh", "emphasize", "question", "emoji"} {
		for sender := 0; sender < 3; sender++ {
			fields := map[string]any{"message_id": reference("messages.message", messageKey(0, 50+i)), "part_index": 0, "sender_handle_id": reference("messages.handle", handleKey(sender)), "is_from_me": sender == 0, "kind": kind, "removed": false, "sent_at": Epoch.Add(-time.Duration(60-i) * time.Minute)}
			if kind == "emoji" {
				fields["emoji"] = "🪁"
			}
			d.put("messages.reaction", fmt.Sprintf("demo-reaction-%02d-%d", i, sender), fields)
		}
	}
	d.put("messages.reaction", "demo-reaction-removed", map[string]any{"message_id": reference("messages.message", messageKey(0, 57)), "part_index": 0, "sender_handle_id": reference("messages.handle", handleKey(4)), "kind": "love", "removed": true, "is_from_me": false, "sent_at": Epoch.Add(-time.Minute)})
}
