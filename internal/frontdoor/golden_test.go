package frontdoor

import (
	"context"
	"encoding/json"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/JohanSardar/apple-bridge/contracts/golden"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestMCPGolden(t *testing.T) {
	for _, mode := range []string{"json", "sse"} {
		deps := Deps{}
		if mode == "sse" {
			deps.PublicHost = func() string { return "fixture.example.com" }
		}
		handler := mcpHandler(deps, contracts.Operations)
		transcript := []any{}
		for _, body := range []string{`{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}}`, `{"jsonrpc":"2.0","method":"notifications/initialized"}`, `{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}`, `{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"records_query","arguments":{"collection":"contacts.contact"}}}`} {
			req := httptest.NewRequest("POST", "http://localhost/mcp", strings.NewReader(body)).WithContext(context.Background())
			req.Header.Set("Content-Type", "application/json")
			req.Header.Set("Accept", "application/json, text/event-stream")
			req.Header.Set("MCP-Protocol-Version", "2025-03-26")
			response := httptest.NewRecorder()
			handler.ServeHTTP(response, req)
			if response.Code >= 400 {
				t.Fatalf("MCP %d %s", response.Code, response.Body.String())
			}
			transcript = append(transcript, map[string]any{"request": body, "status": response.Code, "content_type": response.Header().Get("Content-Type"), "response": response.Body.String()})
		}
		b, e := json.MarshalIndent(map[string]any{"impl": "go", "sdk": "go-sdk v1.8.0", "exchanges": transcript}, "", "  ")
		if e != nil {
			t.Fatal(e)
		}
		golden.Check(t, "mcp-"+mode+".json", append(b, '\n'))
	}
}
