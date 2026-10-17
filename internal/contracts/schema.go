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
	if d.Kind == CommandOp {
		properties := s["properties"].(map[string]any)
		args := JSONSchema(ReminderArgs{})
		fields := args["properties"].(map[string]any)
		required := []string{"id"}
		allowed := map[string]bool{}
		for _, field := range ReminderArgumentFields(d.Name) {
			allowed[field] = true
		}
		switch d.Name {
		case "reminders.create":
			required = []string{"list_id", "title"}
		case "reminders.create_list":
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
		if d.Name != "reminders.create" && d.Name != "reminders.create_list" {
			s["required"] = []string{"args", "expected_version"}
		}
		properties["command_id"].(map[string]any)["description"] = "Optional cmd_<cursor.epoch>_<unique> ID. Omit to generate one; retry the original ID after a lost response. Old epochs are lookup only."

		properties["wait_ms"].(map[string]any)["maximum"] = 15000
		properties["wait_ms"].(map[string]any)["minimum"] = 0
	}
	return s
}
