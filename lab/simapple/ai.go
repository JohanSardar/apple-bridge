package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func simulateAIQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "ai.local" && op != "ai.rewrite" {
		return "", nil, false
	}
	var args struct {
		Prompt string          `json:"prompt"`
		Text   string          `json:"text"`
		Style  string          `json:"style"`
		Schema json.RawMessage `json:"schema"`
	}
	if contracts.DecodeInput(raw, &args) != nil || len(args.Style) > 1024 {
		return "invalid", nil, true
	}
	text := args.Text
	if op == "ai.local" {
		text = args.Prompt
		if args.Text != "" || args.Style != "" {
			return "invalid", nil, true
		}
	} else if args.Prompt != "" || len(args.Schema) > 0 {
		return "invalid", nil, true
	}
	if len(args.Schema) > 0 && string(args.Schema) != "null" {
		var schema struct {
			Type       string `json:"type"`
			Properties map[string]struct {
				Type string `json:"type"`
			} `json:"properties"`
			Required   []string `json:"required"`
			Additional *bool    `json:"additionalProperties"`
		}
		if contracts.DecodeInput(args.Schema, &schema) != nil || schema.Type != "object" || len(schema.Properties) == 0 || len(schema.Properties) > 16 || schema.Required == nil || schema.Additional != nil && *schema.Additional {
			return "invalid", nil, true
		}
		for name, property := range schema.Properties {
			if name == "" || len(name) > 128 {
				return "invalid", nil, true
			}
			switch property.Type {
			case "string", "integer", "number", "boolean":
			default:
				return "invalid", nil, true
			}
		}
		for _, name := range schema.Required {
			if _, ok := schema.Properties[name]; !ok {
				return "invalid", nil, true
			}
		}
	}
	if text == "" || len(text) > 16384 {
		return "invalid", nil, true
	}
	return "", json.RawMessage(`{"status":"unavailable","reason":"Foundation Models unavailable in simulator"}`), true
}
