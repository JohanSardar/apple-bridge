package scenarios

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelAttachments(ctx context.Context, w *world.World, listID string, add func(string, bool)) {
	for _, fixture := range []struct{ key, owner string }{{"own", "rm-rem-parent"}, {"other", "rm-rem-future"}} {
		data := map[string]any{"reminder_id": rmRef("reminders.reminder", fixture.owner), "filename": "Bridge Test " + fixture.key + ".png", "uti": "public.png", "size": 0, "media": nil}
		add("attachment_fixture_"+fixture.key, w.ControlAction("/external-change", map[string]any{"collection": "reminders.attachment", "key": "rm-rem-attachment-" + fixture.key, "data": data}) == nil)
	}
	add("attachments_converged", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
	for _, protocol := range []string{"rest", "mcp"} {
		var page struct {
			Sections []struct {
				Items []struct {
					Reminder    contracts.Record
					Attachments []contracts.Record
				}
			}
		}
		err := (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "reminders.items", map[string]any{"list": listID, "time_zone": "America/New_York"}, &page)
		valid := false
		for _, section := range page.Sections {
			for _, item := range section.Items {
				if recordTitle(item.Reminder) == "Bridge Test Parent" {
					if len(item.Attachments) == 1 {
						var attachment struct {
							ReminderID string `json:"reminder_id"`
							Filename   string `json:"filename"`
						}
						err := json.Unmarshal(item.Attachments[0].Data, &attachment)
						valid = err == nil && item.Attachments[0].Collection == "reminders.attachment" && attachment.ReminderID == item.Reminder.ID && attachment.Filename == "Bridge Test own.png"
					}
				}
			}
		}
		add(protocol+"_attachments_are_owned_records", err == nil && valid)
	}
}
