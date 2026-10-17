package contracts

import (
	"encoding/json"
	"reflect"
	"strings"
	"time"
)

func JSONSchema(value any) map[string]any { return schemaType(reflect.TypeOf(value)) }
func schemaType(t reflect.Type) map[string]any {
	if t == nil {
		return map[string]any{}
	}
	if t.Kind() == reflect.Pointer {
		return schemaType(t.Elem())
	}
	if t == reflect.TypeFor[time.Time]() {
		return map[string]any{"type": "string", "format": "date-time"}
	}
	if t == reflect.TypeFor[json.RawMessage]() {
		return map[string]any{"type": "object"}
	}
	switch t.Kind() {
	case reflect.Struct:
		properties := map[string]any{}
		required := []string{}
		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)
			tag := strings.Split(f.Tag.Get("json"), ",")
			name := tag[0]
			if name == "-" {
				continue
			}
			if name == "" {
				name = f.Name
			}
			properties[name] = schemaType(f.Type)
			if !strings.Contains(f.Tag.Get("json"), "omitempty") {
				required = append(required, name)
			}
		}
		return map[string]any{"type": "object", "properties": properties, "required": required, "additionalProperties": false}
	case reflect.Slice, reflect.Array:
		return map[string]any{"type": "array", "items": schemaType(t.Elem())}
	case reflect.Bool:
		return map[string]any{"type": "boolean"}
	case reflect.Int, reflect.Int64, reflect.Uint64, reflect.Int32:
		return map[string]any{"type": "integer"}
	case reflect.Float32, reflect.Float64:
		return map[string]any{"type": "number"}
	case reflect.Map:
		return map[string]any{"type": "object"}
	default:
		return map[string]any{"type": "string"}
	}
}
func OperationSchema(d OperationDescriptor) map[string]any {
	s := JSONSchema(d.Input)
	if strings.HasPrefix(d.Name, "notes.") {
		return noteSchema(d, s)
	}
	if d.Kind == CommandOp {
		properties := s["properties"].(map[string]any)
		args := JSONSchema(ReminderArgs{})
		if d.Source == Calendar {
			args = JSONSchema(CalendarArgs{})
		}
		fields := args["properties"].(map[string]any)
		required := []string{"id"}
		allowed := map[string]bool{}
		argumentFields := ReminderArgumentFields(d.Name)
		if d.Source == Calendar {
			argumentFields = CalendarArgumentFields(d.Name)
		}
		for _, field := range argumentFields {
			allowed[field] = true
		}
		switch d.Name {
		case "calendar.create_event":
			required = []string{"calendar_id", "title", "start", "end"}
		case "calendar.update_event", "calendar.delete_event":
			required = []string{"id", "span"}
			fields["span"].(map[string]any)["enum"] = []string{"this", "future"}
		case "reminders.create":
			required = []string{"list_id", "title"}
		case "calendar.create_calendar", "reminders.create_list":
			required = []string{"title"}
		case "reminders.update":
			args["minProperties"] = 2
		}
		for name := range fields {
			if !allowed[name] {
				delete(fields, name)
			}
		}
		args["required"] = required
		properties["args"] = args
		if d.Name != "reminders.create" && d.Name != "reminders.create_list" && d.Name != "calendar.create_event" && d.Name != "calendar.create_calendar" {
			s["required"] = []string{"args", "expected_version"}
		}
		commandID := properties["command_id"].(map[string]any)
		commandID["description"] = "Optional opaque caller ID, independent of replication cursors. Omit to generate one; retry the original ID after a lost response. IDs from older admission eras are lookup only."
		commandID["minLength"] = 1
		commandID["maxLength"] = 128
		commandID["pattern"] = "^[A-Za-z0-9._:-]+$"

		properties["wait_ms"].(map[string]any)["maximum"] = 15000
		properties["wait_ms"].(map[string]any)["minimum"] = 0
	}
	return s
}

func noteSchema(d OperationDescriptor, schema map[string]any) map[string]any {
	properties := schema["properties"].(map[string]any)
	args := JSONSchema(NoteArgs{})
	fields := args["properties"].(map[string]any)
	allowed := map[string]bool{}
	required := []string{"id"}
	switch d.Name {
	case "notes.create":
		required = []string{"folder_id", "title"}
		allowed = map[string]bool{"folder_id": true, "title": true, "plain_text": true}
	case "notes.create_folder":
		required = []string{"title"}
		allowed = map[string]bool{"title": true}
	case "notes.update":
		required = []string{"id", "plain_text"}
		allowed = map[string]bool{"id": true, "plain_text": true, "mode": true}
		fields["mode"].(map[string]any)["enum"] = []string{"replace", "append"}
	case "notes.delete":
		allowed = map[string]bool{"id": true}
	}
	for field := range fields {
		if !allowed[field] {
			delete(fields, field)
		}
	}
	args["required"] = required
	properties["args"] = args
	schema["required"] = []string{"args"}
	if d.Name == "notes.update" || d.Name == "notes.delete" {
		schema["required"] = []string{"args", "expected_version"}
	}
	properties["wait_ms"].(map[string]any)["maximum"] = 15000
	properties["wait_ms"].(map[string]any)["minimum"] = 0
	return schema
}
