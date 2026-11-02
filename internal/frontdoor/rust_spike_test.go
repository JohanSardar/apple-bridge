package frontdoor

import (
	"bufio"
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func TestRustMCPParitySpike(t *testing.T) {
	binary := os.Getenv("AB_SPIKE_BINARY")
	if binary == "" {
		t.Skip("AB_SPIKE_BINARY is unset")
	}
	root, err := filepath.Abs("../..")
	if err != nil {
		t.Fatal(err)
	}
	for _, mode := range []string{"json", "sse"} {
		t.Run(mode, func(t *testing.T) {
			deps := Deps{}
			if mode == "sse" {
				deps.PublicHost = func() string { return "fixture.example.com" }
			}
			goServer := httptest.NewServer(mcpHandler(deps, contracts.Operations))
			defer goServer.Close()
			cmd := exec.Command(binary, "serve", mode)
			cmd.Dir = root
			out, err := cmd.StdoutPipe()
			if err != nil {
				t.Fatal(err)
			}
			if err := cmd.Start(); err != nil {
				t.Fatal(err)
			}
			defer func() { cmd.Process.Kill(); cmd.Wait() }()
			scanner := bufio.NewScanner(out)
			if !scanner.Scan() {
				t.Fatal("no address")
			}
			rustURL := "http://" + scanner.Text()
			cases := []string{
				`{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nonexistent","arguments":{}}}`,
				`{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"records_query","arguments":{}}}`,
				`{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"records_query","arguments":{"collection":123}}}`,
				`{"jsonrpc":"2.0","id":4,"method":"nonexistent","params":{}}`,
				`{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":99}}`,
				`{`,
			}
			for _, version := range []string{"2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"} {
				for _, body := range append([]string{`{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"` + version + `","capabilities":{},"clientInfo":{"name":"synthetic","version":"1"}}}`}, cases...) {
					results := []any{}
					for _, endpoint := range []string{goServer.URL, rustURL} {
						req, err := http.NewRequest("POST", endpoint+"/mcp", strings.NewReader(body))
						if err != nil {
							t.Fatal(err)
						}
						req.Header.Set("Content-Type", "application/json")
						req.Header.Set("Accept", "application/json, text/event-stream")
						req.Header.Set("MCP-Protocol-Version", version)
						response, err := http.DefaultClient.Do(req)
						if err != nil {
							t.Fatal(err)
						}
						b, err := io.ReadAll(response.Body)
						response.Body.Close()
						if err != nil {
							t.Fatal(err)
						}
						if strings.Contains(response.Header.Get("Content-Type"), "text/event-stream") {
							for _, line := range strings.Split(string(b), "\n") {
								if strings.HasPrefix(line, "data:") {
									b = []byte(strings.TrimSpace(line[5:]))
								}
							}
						}
						var value any
						if json.Unmarshal(b, &value) != nil {
							value = string(bytes.TrimSpace(b))
						}
						results = append(results, map[string]any{"status": response.StatusCode, "value": value})
					}
					a, _ := json.Marshal(results[0])
					b, _ := json.Marshal(results[1])
					if !bytes.Equal(a, b) {
						if body != cases[3] && body != cases[5] {
							t.Fatalf("unexpected parity difference: %s vs %s", a, b)
						}
						t.Logf("DIFF version=%s request=%s go=%s rust=%s", version, body, a, b)
					}
				}
			}
		})
	}
}
