package contracts

import (
	"encoding/json"
	"sort"
	"strings"
	"time"
)

type fieldKind uint8

const (
	shapeString fieldKind = iota + 1
	shapeTime
	shapeBool
	shapeNumber
	shapeArray
	shapeObject
)

type fieldShape struct {
	kind     fieldKind
	required bool
	nullable bool
}

func requiredField(kind fieldKind) fieldShape { return fieldShape{kind: kind, required: true} }
func optionalField(kind fieldKind) fieldShape { return fieldShape{kind: kind} }
func nullableField(kind fieldKind) fieldShape { return fieldShape{kind: kind, nullable: true} }

var messagesShape = map[string]map[string]fieldShape{
	"messages.message": {
		"chat_id": optionalField(shapeString), "sender_handle_id": optionalField(shapeString), "is_from_me": requiredField(shapeBool), "service": requiredField(shapeString),
		"sent_at": requiredField(shapeTime), "edit_until": optionalField(shapeTime), "unsend_until": optionalField(shapeTime), "parts": requiredField(shapeArray), "text": optionalField(shapeString),
		"decode_status": requiredField(shapeString), "raw_body_digest": optionalField(shapeString), "summary_decode_status": optionalField(shapeString), "summary_raw_digest": optionalField(shapeString),
		"reply_to_id": optionalField(shapeString), "thread_originator_id": optionalField(shapeString), "edited_at": optionalField(shapeTime), "edit_history": requiredField(shapeArray),
		"retracted_parts": requiredField(shapeArray), "retracted_at": optionalField(shapeTime), "delivered_at": optionalField(shapeTime), "read_at": optionalField(shapeTime),
		"error_code": requiredField(shapeNumber), "effect": optionalField(shapeString), "kind": requiredField(shapeString), "group_event": optionalField(shapeObject), "balloon": optionalField(shapeObject),
		"audio": requiredField(shapeObject), "schedule": optionalField(shapeObject), "subject": optionalField(shapeString), "delivery": requiredField(shapeObject), "is_spam": requiredField(shapeBool),
		"has_unseen_mention": requiredField(shapeBool), "via_satellite": requiredField(shapeBool), "is_sos": requiredField(shapeBool), "formatting": requiredField(shapeArray),
		"deleted_at": optionalField(shapeTime), "recoverable_until": nullableField(shapeTime),
	},
	"messages.chat": {
		"service": requiredField(shapeString), "style": requiredField(shapeString), "display_name": requiredField(shapeString), "participants": requiredField(shapeArray),
		"is_archived": requiredField(shapeBool), "last_read_at": optionalField(shapeTime), "is_unread": optionalField(shapeBool), "capabilities": requiredField(shapeArray),
		"filter": requiredField(shapeString), "is_spam": requiredField(shapeBool), "group_photo_attachment_id": optionalField(shapeString), "read_receipts": requiredField(shapeString),
		"muted": optionalField(shapeBool), "muted_until": nullableField(shapeTime), "pinned_index": nullableField(shapeNumber), "deleted_at": optionalField(shapeTime), "recoverable_until": nullableField(shapeTime),
	},
	"messages.handle": {
		"address": requiredField(shapeString), "service": requiredField(shapeString), "contact_id": optionalField(shapeString),
	},
	"messages.reaction": {
		"message_id": requiredField(shapeString), "part_index": requiredField(shapeNumber), "sender_handle_id": optionalField(shapeString), "is_from_me": requiredField(shapeBool),
		"kind": requiredField(shapeString), "emoji": optionalField(shapeString), "removed": requiredField(shapeBool), "sent_at": requiredField(shapeTime),
	},
	"messages.attachment": {
		"message_id": optionalField(shapeString), "filename": requiredField(shapeString), "mime_type": requiredField(shapeString), "uti": requiredField(shapeString), "size": requiredField(shapeNumber),
		"blob": requiredField(shapeObject), "derivatives": optionalField(shapeObject), "media": optionalField(shapeObject), "is_sticker": requiredField(shapeBool), "sticker": optionalField(shapeObject),
		"hidden": requiredField(shapeBool), "genmoji": requiredField(shapeBool), "description": optionalField(shapeString),
	},
}

type ShapeViolation struct {
	Collection string `json:"collection"`
	Field      string `json:"field"`
	Problem    string `json:"problem"`
}

func (v ShapeViolation) Key() string { return v.Collection + "." + v.Field + ": " + v.Problem }

func HasShape(collection string) bool { _, ok := messagesShape[collection]; return ok }

func CheckShape(collection string, data json.RawMessage) []ShapeViolation {
	fields, ok := messagesShape[collection]
	if !ok {
		return nil
	}
	var values map[string]any
	if json.Unmarshal(data, &values) != nil || values == nil {
		return []ShapeViolation{{Collection: collection, Field: "data", Problem: "not_object"}}
	}
	out := []ShapeViolation{}
	for name, value := range values {
		field, known := fields[name]
		switch {
		case !known:
			out = append(out, ShapeViolation{collection, name, "unknown_field"})
		case value == nil && !field.nullable:
			out = append(out, ShapeViolation{collection, name, "unexpected_null"})
		case value != nil && !field.accepts(value):
			out = append(out, ShapeViolation{collection, name, "wrong_type"})
		}
	}
	for name, field := range fields {
		if _, present := values[name]; field.required && !present {
			out = append(out, ShapeViolation{collection, name, "missing_field"})
		}
	}
	sort.Slice(out, func(i, j int) bool { return out[i].Key() < out[j].Key() })
	return out
}

func (f fieldShape) accepts(value any) bool {
	switch f.kind {
	case shapeString:
		_, ok := value.(string)
		return ok
	case shapeTime:
		text, ok := value.(string)
		if !ok {
			return false
		}
		_, err := time.Parse(time.RFC3339Nano, text)
		return err == nil && strings.Contains(text, "T")
	case shapeBool:
		_, ok := value.(bool)
		return ok
	case shapeNumber:
		_, ok := value.(float64)
		return ok
	case shapeArray:
		_, ok := value.([]any)
		return ok
	case shapeObject:
		_, ok := value.(map[string]any)
		return ok
	}
	return false
}
