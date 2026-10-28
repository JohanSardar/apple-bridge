package contracts

import (
	"encoding/json"
	"reflect"
	"slices"
	"strings"
	"time"
)

type schemaBuilder struct {
	active      map[reflect.Type]bool
	referenced  map[reflect.Type]bool
	definitions map[string]any
}

func JSONSchema(value any) map[string]any {
	builder := schemaBuilder{active: map[reflect.Type]bool{}, referenced: map[reflect.Type]bool{}, definitions: map[string]any{}}
	schema := builder.build(reflect.TypeOf(value))
	if _, ok := value.(QueryResult); ok {
		schema["properties"].(map[string]any)["result_json"] = map[string]any{}
	}
	if len(builder.definitions) > 0 {
		schema["$defs"] = builder.definitions
		schema["$id"] = "urn:apple-bridge:schema:" + reflect.TypeOf(value).String()
	}
	return schema
}

func (builder *schemaBuilder) build(t reflect.Type) map[string]any {
	if t == nil {
		return map[string]any{}
	}
	if t.Kind() == reflect.Pointer {
		schema := builder.build(t.Elem())
		schema["nullable"] = true
		return schema
	}
	if t == reflect.TypeFor[time.Time]() {
		return map[string]any{"type": "string", "format": "date-time"}
	}
	if t == reflect.TypeFor[json.RawMessage]() {
		return map[string]any{"type": "object"}
	}
	switch t.Kind() {
	case reflect.Struct:
		if builder.active[t] {
			builder.referenced[t] = true
			return map[string]any{"$ref": "#/$defs/" + t.String()}
		}
		builder.active[t] = true
		defer delete(builder.active, t)
		properties := map[string]any{}
		required := []string{}
		for i := 0; i < t.NumField(); i++ {
			f := t.Field(i)
			if f.Anonymous && f.Type.Kind() == reflect.Struct {
				embedded := builder.build(f.Type)
				for name, value := range embedded["properties"].(map[string]any) {
					properties[name] = value
				}
				required = append(required, embedded["required"].([]string)...)
				continue
			}
			tag := strings.Split(f.Tag.Get("json"), ",")
			name := tag[0]
			if name == "-" {
				continue
			}
			if name == "" {
				name = f.Name
			}
			field := builder.build(f.Type)
			if description := f.Tag.Get("description"); description != "" {
				field["description"] = description
			}
			if enum := f.Tag.Get("enum"); enum != "" {
				field["enum"] = strings.Split(enum, ",")
			}
			properties[name] = field
			if !strings.Contains(f.Tag.Get("json"), "omitempty") {
				required = append(required, name)
			}
		}
		schema := map[string]any{"type": "object", "properties": properties, "required": required, "additionalProperties": false}
		if builder.referenced[t] {
			builder.definitions[t.String()] = schema
		}
		return schema
	case reflect.Slice, reflect.Array:
		return map[string]any{"type": "array", "items": builder.build(t.Elem())}
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
	if d.Kind == QueryOp {
		props := s["properties"].(map[string]any)
		props["args"] = map[string]any{}
		props["id"] = map[string]any{"type": "string", "pattern": "^qry_[A-Za-z0-9._:-]+$", "maxLength": 128}
		props["deadline_ms"] = map[string]any{"type": "integer", "minimum": 0, "maximum": 60000}
		props["op"] = map[string]any{"type": "string", "enum": []string{d.Name}}
		if d.Name == "helper.version" {
			props["args"] = map[string]any{"type": "object", "additionalProperties": false}
		}
		if d.Name == "test.sleep" || d.Name == "test.blob" {
			field, max := "ms", 60000
			if d.Name == "test.blob" {
				field, max = "size", 64<<20
			}
			props["args"] = map[string]any{"type": "object", "required": []string{field}, "additionalProperties": false, "properties": map[string]any{field: map[string]any{"type": "integer", "minimum": 0, "maximum": max}}}
		}
		return s
	}
	if _, ok := d.Input.(ReadQuery); ok {
		collections := []string{""}
		for collection := range Collections {
			collections = append(collections, collection)
		}
		slices.Sort(collections)
		fields := s["properties"].(map[string]any)
		collection := fields["collection"].(map[string]any)
		collection["enum"] = collections
		collection["description"] = "Collection name; use an empty string to query all visible collections. Follow next with after to read further pages."
	}
	if d.Name == "messages.handles" {
		s["properties"].(map[string]any)["ids"].(map[string]any)["maxItems"] = 500
		return s
	}
	if d.Name == "messages.chats" || d.Name == "messages.thread" || d.Name == "messages.collection" || d.Name == "messages.search" {
		fields := s["properties"].(map[string]any)
		if d.Name == "messages.search" {
			fields["text"].(map[string]any)["maxLength"] = 512
		}
		if d.Name == "messages.chats" {
			fields["participants"].(map[string]any)["minItems"] = 1
			fields["participants"].(map[string]any)["maxItems"] = 32
		}
		fields["limit"].(map[string]any)["minimum"] = 0
		fields["limit"].(map[string]any)["maximum"] = 200
		return s
	}
	if d.Kind == CommandOp && strings.HasPrefix(d.Name, "notes.") {
		return noteSchema(d, s)
	}
	if d.Kind == CommandOp {
		properties := s["properties"].(map[string]any)
		args := JSONSchema(ReminderArgs{})
		if d.Source == Calendar {
			args = JSONSchema(CalendarArgs{})
		}
		if d.Source == Messages {
			args = JSONSchema(MessagesArgs{})
		}
		fields := args["properties"].(map[string]any)
		required := []string{"id"}
		allowed := map[string]bool{}
		argumentFields := ReminderArgumentFields(d.Name)
		if d.Source == Calendar {
			argumentFields = CalendarArgumentFields(d.Name)
		}
		if d.Source == Messages {
			argumentFields = MessagesArgumentFields(d.Name)
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
		if d.Source == Messages {
			required = []string{}
			if d.Name != "messages.send" && d.Name != "messages.create_group" {
				required = []string{"id"}
			}
		}
		if d.Source == Messages {
			switch d.Name {
			case "messages.reply", "messages.edit":
				required = []string{"id", "text"}
			case "messages.react":
				required = []string{"id", "reaction"}
			case "messages.create_group":
				required = []string{"handles", "service", "text"}
			}
			if service, ok := fields["service"].(map[string]any); ok {
				service["enum"] = []string{"iMessage", "SMS", "RCS"}
			}
			if reaction, ok := fields["reaction"].(map[string]any); ok {
				reaction["enum"] = []string{"love", "like", "dislike", "laugh", "emphasize", "question", "emoji"}
			}
			if part, ok := fields["part_index"].(map[string]any); ok {
				part["minimum"] = 0
			}
			if d.Name == "messages.send" {
				args["oneOf"] = []any{map[string]any{"required": []string{"chat_id"}}, map[string]any{"required": []string{"handles", "service"}}}
				args["anyOf"] = []any{map[string]any{"required": []string{"text"}}, map[string]any{"required": []string{"files"}}}
			}
			if d.Name == "messages.edit" || d.Name == "messages.unsend" || d.Name == "messages.keep_audio" || strings.HasPrefix(d.Name, "messages.delete_") {
				s["required"] = []string{"args", "expected_version"}
			}
		}
		args["required"] = required
		properties["args"] = args
		if d.Source == Messages {
			applyMessagesParitySchema(d.Name, s, args)
		}
		if d.Source != Messages && d.Name != "reminders.create" && d.Name != "reminders.create_list" && d.Name != "calendar.create_event" && d.Name != "calendar.create_calendar" {
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
