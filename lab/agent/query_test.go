package agent

import (
	"strings"
	"testing"
)

func TestQueryChecks(t *testing.T) {
	valid := `{"type":"item.completed","item":{"type":"mcp_tool_call","server":"apple_bridge","tool":"maps_eta","arguments":{"args":{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8024,"lon":-122.4058},"mode":"driving"}},"result":{"isError":false,"structuredContent":{"result_json":{"distance":1800,"duration":600000}}}}}`
	answer := "maps.eta synthetic: 600,000 milliseconds, 1,800 meters"
	cases := []struct {
		name, transcript, answer string
		tools, used              bool
	}{
		{"valid", valid, answer, true, true},
		{"missing_call", "", answer, false, false},
		{"wrong_destination", strings.ReplaceAll(valid, "37.8024", "37.9"), answer, false, false},
		{"wrong_mode", strings.ReplaceAll(valid, "driving", "walking"), answer, false, false},
		{"wrong_server", strings.ReplaceAll(valid, "apple_bridge", "other"), answer, false, false},
		{"tool_error", strings.ReplaceAll(valid, `"isError":false`, `"isError":true`), answer, false, false},
		{"execution_error", strings.ReplaceAll(valid, `"result":`, `"error":{"message":"failed"},"result":`), answer, false, false},
		{"started_call", strings.ReplaceAll(valid, "item.completed", "item.started"), answer, false, false},
		{"missing_result", strings.ReplaceAll(valid, `"result_json"`, `"missing"`), answer, false, false},
		{"changed_duration", strings.ReplaceAll(valid, "600000", "700000"), answer, true, false},
		{"changed_distance", strings.ReplaceAll(valid, "1800", "2000"), answer, true, false},
		{"wrong_units", valid, strings.ReplaceAll(answer, "milliseconds", "seconds"), true, false},
		{"missing_citation", valid, "synthetic: 600000 ms, 1800 m", true, false},
		{"missing_synthetic", valid, "maps.eta: 600000 ms, 1800 m", true, false},
		{"short_units", valid, "maps.eta synthetic: 600000 ms, 1800 m", true, true},
		{"text_result", strings.ReplaceAll(valid, `"structuredContent":{"result_json":{"distance":1800,"duration":600000}}`, `"content":[{"type":"text","text":"{\"result_json\":{\"distance\":1800,\"duration\":600000}}"}]`), answer, true, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			tools, used := queryChecks([]byte(c.transcript), c.answer)
			if tools != c.tools || used != c.used {
				t.Fatalf("tools=%t answer=%t want tools=%t answer=%t", tools, used, c.tools, c.used)
			}
		})
	}
}
