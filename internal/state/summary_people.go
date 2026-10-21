package state

import (
	"context"
	"database/sql"
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func summaryPeople(ctx context.Context, tx *sql.Tx, visibility trust.Visibility, chats []contracts.ChatSummary) (map[string]contracts.Record, error) {
	people := map[string]contracts.Record{}
	ids := map[string]bool{}
	for _, chat := range chats {
		participants, _ := modelFields(chat.Chat)["participants"].([]any)
		for _, participant := range participants {
			if id, ok := participant.(string); ok {
				ids[id] = true
			}
		}
	}
	for _, collection := range []string{"messages.handle", "contacts.contact"} {
		if collection == "contacts.contact" && !visibility[contracts.Contacts] {
			break
		}
		if len(ids) == 0 {
			break
		}
		args := []any{collection}
		for id := range ids {
			args = append(args, id)
		}
		records, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection=? AND deleted=0 AND id IN ("+strings.TrimSuffix(strings.Repeat("?,", len(ids)), ",")+")", args...)
		if err != nil {
			return nil, err
		}
		ids = map[string]bool{}
		for _, record := range records {
			people[collection+"/"+record.ID] = record
			if id := fieldString(modelFields(record), "contact_id"); id != "" {
				ids[id] = true
			}
		}
	}
	return people, nil
}

type summaryActivity struct {
	message     contracts.Record
	activityID  string
	attachments []contracts.Record
}

func summaryActivities(ctx context.Context, tx *sql.Tx, chats []contracts.ChatSummary) (map[string]summaryActivity, error) {
	out := map[string]summaryActivity{}
	if len(chats) == 0 {
		return out, nil
	}
	args := []any{}
	for _, chat := range chats {
		args = append(args, chat.Chat.ID)
	}
	slots := strings.TrimSuffix(strings.Repeat("?,", len(args)), ",")
	rows, err := tx.QueryContext(ctx, "SELECT c.id,a.id,r.payload FROM chat_summary c JOIN activity_by_chat a ON a.id=(SELECT id FROM activity_by_chat latest WHERE latest.chat_id=c.id ORDER BY latest.at DESC,latest.id DESC LIMIT 1) JOIN records r ON r.collection='messages.message' AND r.id=a.message_id WHERE c.id IN ("+slots+")", args...)
	if err != nil {
		return nil, err
	}
	messageChats := map[string][]string{}
	for rows.Next() {
		var id, activityID string
		var raw []byte
		var message contracts.Record
		if err = rows.Scan(&id, &activityID, &raw); err != nil {
			break
		}
		if err = json.Unmarshal(raw, &message); err != nil {
			break
		}
		out[id] = summaryActivity{message: message, activityID: activityID}
		messageChats[message.ID] = append(messageChats[message.ID], id)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		return nil, err
	}
	if len(messageChats) == 0 {
		return out, nil
	}
	args = nil
	for id := range messageChats {
		args = append(args, id)
	}
	slots = strings.TrimSuffix(strings.Repeat("?,", len(args)), ",")
	attachments, err := readModelRecords(ctx, tx, "SELECT payload FROM attachment_by_message WHERE message_id IN ("+slots+") AND coalesce(json_extract(payload,'$.data.hidden'),0)=0 ORDER BY id", args...)
	if err != nil {
		return nil, err
	}
	for _, attachment := range attachments {
		for _, id := range messageChats[attachmentMessage(modelFields(attachment))] {
			entry := out[id]
			entry.attachments = append(entry.attachments, attachment)
			out[id] = entry
		}
	}
	return out, nil
}
