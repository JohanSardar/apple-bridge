package state

import (
	"context"
	"database/sql"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"strings"
)

func summaryPinPreviews(ctx context.Context, tx *sql.Tx, chats []contracts.ChatSummary) (map[string]*contracts.PinPreview, error) {
	out := map[string]*contracts.PinPreview{}
	args := []any{}
	for _, chat := range chats {
		if chat.UnreadCount > 0 {
			args = append(args, chat.Chat.ID)
		}
	}
	if len(args) == 0 {
		return out, nil
	}
	slots := strings.TrimSuffix(strings.Repeat("?,", len(args)), ",")
	messages, err := readModelRecords(ctx, tx, "SELECT m.payload FROM chat_summary c JOIN msg_by_chat m ON m.id=(SELECT id FROM msg_by_chat latest WHERE latest.chat_id=c.id AND latest.unread=1 ORDER BY latest.sent_at DESC,latest.id DESC LIMIT 1) WHERE c.id IN ("+slots+")", args...)
	if err != nil {
		return nil, err
	}
	args = nil
	for _, message := range messages {
		if fieldString(modelFields(message), "text") == "" {
			args = append(args, message.ID)
		}
	}
	attachments := map[string][]contracts.Record{}
	if len(args) > 0 {
		slots = strings.TrimSuffix(strings.Repeat("?,", len(args)), ",")
		records, err := readModelRecords(ctx, tx, "SELECT payload FROM attachment_by_message WHERE message_id IN ("+slots+") AND coalesce(json_extract(payload,'$.data.hidden'),0)=0 ORDER BY id", args...)
		if err != nil {
			return nil, err
		}
		for _, record := range records {
			id := attachmentMessage(modelFields(record))
			attachments[id] = append(attachments[id], record)
		}
	}
	for _, message := range messages {
		data := modelFields(message)
		out[fieldString(data, "chat_id")] = &contracts.PinPreview{MessageID: message.ID, TextPreview: pinMessageText(message, attachments[message.ID]), At: fieldString(data, "sent_at")}
	}
	return out, nil
}

func pinMessageText(message contracts.Record, attachments []contracts.Record) string {
	data := modelFields(message)
	if text := fieldString(data, "text"); text != "" {
		runes := []rune(text)
		if len(runes) > 200 {
			return string(runes[:200]) + "…"
		}
		return text
	}
	if len(attachments) > 0 {
		images := true
		for _, attachment := range attachments {
			images = images && attachmentKind(modelFields(attachment)) == "photos"
		}
		if images {
			noun := "Images"
			if len(attachments) == 1 {
				noun = "Image"
			}
			return fmt.Sprintf("Attachment: %d %s", len(attachments), noun)
		}
		if len(attachments) == 1 {
			if strings.Contains(strings.ToLower(fieldString(modelFields(attachments[0]), "mime_type")), "vcard") {
				return "Contact Card"
			}
			if label := map[string]string{"videos": "Video", "audio": "Audio Message", "stickers": "Sticker"}[attachmentKind(modelFields(attachments[0]))]; label != "" {
				return label
			}
		}
		noun := "Files"
		if len(attachments) == 1 {
			noun = "File"
		}
		return fmt.Sprintf("Attachment: %d %s", len(attachments), noun)
	}
	balloon, _ := data["balloon"].(map[string]any)
	if fieldString(balloon, "kind") == "link" {
		return "Link"
	}
	if fieldString(balloon, "kind") == "find_my" {
		return "Location"
	}
	return "Message"
}
