package agent

import (
	"strings"
	"testing"
)

func TestQueryTools(t *testing.T) {
	weather := `{"type":"item.completed","item":{"type":"mcp_tool_call","server":"apple_bridge","tool":"weather_current","arguments":{"args":{"latitude":37.7955,"longitude":-122.3937}},"result":{"isError":false}}}`
	eta := `{"type":"item.completed","item":{"type":"mcp_tool_call","server":"apple_bridge","tool":"maps_eta","arguments":{"args":{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8024,"lon":-122.4058},"mode":"driving"}},"result":{"isError":false}}}`
	valid := weather + "\n" + eta
	cases := []struct {
		name, transcript string
		want             bool
	}{
		{"both", valid, true},
		{"missing_eta", weather, false},
		{"wrong_destination", strings.ReplaceAll(valid, "37.8024", "37.9"), false},
		{"wrong_mode", strings.ReplaceAll(valid, "driving", "walking"), false},
		{"wrong_server", strings.ReplaceAll(valid, "apple_bridge", "other"), false},
		{"tool_error", strings.ReplaceAll(valid, `"isError":false`, `"isError":true`), false},
		{"missing_result", strings.ReplaceAll(valid, `{"isError":false}`, `null`), false},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := queryTools([]byte(c.transcript)); got != c.want {
				t.Fatalf("tools=%t want=%t", got, c.want)
			}
		})
	}
}
