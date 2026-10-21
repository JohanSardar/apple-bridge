package scenarios

import (
	"context"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"strings"
	"time"
)

func demoReadmodelCoverage(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) []contracts.Record {
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	var chats contracts.ChatsResult
	err := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &chats)
	var archived contracts.ChatsResult
	archiveErr := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: "archived", Limit: 200}, &archived)
	add("messages_chats_37_active_2_archived", err == nil && archiveErr == nil && len(chats.Chats) == 37 && len(archived.Chats) == 2)
	var deleted contracts.ChatsResult
	deletedErr := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Filter: "deleted", Limit: 200}, &deleted)
	add("messages_chats_recently_deleted", deletedErr == nil && len(deleted.Chats) == 1)
	summaries := append(append(chats.Chats, archived.Chats...), deleted.Chats...)
	coverage := map[string]int{}
	mediaRecords := []contracts.Record{}
	messages := 0
	unread := 0
	allRead := true
	for _, chat := range summaries {
		data := rmData(chat.Chat)
		coverage["service_"+fmt.Sprint(data["service"])]++
		coverage["style_"+fmt.Sprint(data["style"])]++
		if data["style"] == "group" {
			if data["display_name"] == "" {
				coverage["unnamed_group"]++
			} else {
				coverage["named_group"]++
			}
		}
		if chat.LastMessage != nil {
			coverage["preview_"+chat.LastMessage.Preview.Kind]++
		}
		if chat.UnreadCount > 0 {
			unread++
		}
		query := contracts.ThreadQuery{ChatID: chat.Chat.ID, Limit: 200}
		seen := map[string]bool{}
		stackSender := ""
		stackAt := time.Time{}
		stackSize := 0
		textSender := ""
		textAt := time.Time{}
		textRun := 0
		for {
			var thread contracts.ThreadResult
			if err = client.Call(ctx, "messages.thread", query, &thread); err != nil {
				allRead = false
				break
			}
			for _, item := range thread.Items {
				if seen[item.ID] {
					allRead = false
				}
				seen[item.ID] = true
				messages++
				demoItemCoverage(item, coverage)
				fields := rmData(item.Record)
				at, _ := time.Parse(time.RFC3339, fmt.Sprint(fields["sent_at"]))
				sender := fmt.Sprint(fields["sender_handle_id"], fields["is_from_me"])
				if fields["text"] != "" && len(item.Attachments) == 0 {
					if sender == textSender && at.Sub(textAt) >= 0 && at.Sub(textAt) <= time.Minute {
						textRun++
					} else {
						textRun = 1
					}
					textSender, textAt = sender, at
					if textRun >= 3 {
						coverage[fmt.Sprint("text_run_", fields["is_from_me"])]++
					}
				} else {
					textRun = 0
					textSender = ""
				}

				imageOnly := len(item.Attachments) > 0
				for _, attachment := range item.Attachments {
					imageOnly = imageOnly && strings.HasPrefix(fmt.Sprint(rmData(attachment)["mime_type"]), "image/")
				}
				if imageOnly {
					if sender == stackSender && at.Sub(stackAt) >= 0 && at.Sub(stackAt) <= time.Minute {
						stackSize++
					} else {
						stackSize = 1
					}
					stackSender, stackAt = sender, at
					if stackSize >= 3 {
						coverage["image_stack_run"]++
					}
				} else {
					stackSize = 0
					stackSender = ""
				}
				mediaRecords = append(mediaRecords, item.Attachments...)
			}
			if thread.Before == "" {
				break
			}
			query.Before = thread.Before
		}
	}
	add("all_5000_messages_visible_without_duplicates", allRead && messages == 5000)
	add("unread_backlogs_in_several_chats", unread >= 3)
	required := []string{"service_iMessage", "service_SMS", "service_RCS", "style_direct", "style_group", "named_group", "unnamed_group", "text", "long_text", "emoji_1", "emoji_2", "emoji_3", "url", "mention", "reply", "nested_reply", "reply_count", "edited", "unsent", "failed", "delivered", "read", "multipart", "group_rename", "group_member_added", "group_member_removed", "group_member_left", "group_photo_changed", "portrait", "landscape", "square", "panorama", "transparent", "gif", "video", "audio", "pdf", "vcard", "image_stack", "image_stack_run", "text_run_true", "text_run_false", "reaction_love", "reaction_like", "reaction_dislike", "reaction_laugh", "reaction_emphasize", "reaction_question", "reaction_emoji", "multiple_reactors", "today", "yesterday", "this_week", "older", "year_change", "preview_text", "preview_attachments", "preview_edited", "preview_unsent", "preview_group_event", "preview_reaction", "image_variants", "video_variants", "audio_variants", "transparent_display_png", "audio_waveform", "gif_thumb"}
	for _, name := range []string{"demo-portrait.png", "demo-landscape.png", "demo-square.png", "demo-panorama.png", "demo-transparent.png", "demo-video.mp4", "demo-ultrawide.mp4"} {
		required = append(required, "geometry_"+name)
	}
	for _, effect := range demo.Effects {
		required = append(required, "effect_"+effect)
	}
	for _, kind := range required {
		add("readmodel_coverage_"+kind, coverage[kind] > 0)
	}
	e.Metrics["coverage"] = coverage
	return mediaRecords
}
