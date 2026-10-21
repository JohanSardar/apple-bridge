package scripted

import (
	"bytes"
	"context"
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type Client struct {
	Endpoint     world.Endpoint
	Protocol     string
	Observe      func(http.Header, []byte)
	DropResponse bool
}

func (c Client) Call(ctx context.Context, name string, input, output any) error {
	body, err := json.Marshal(input)
	if err != nil {
		return err
	}
	path := "/v1/op/" + name
	if c.Protocol == "mcp" {
		body, err = json.Marshal(map[string]any{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": map[string]any{"name": strings.ReplaceAll(name, ".", "_"), "arguments": input}})
		path = "/mcp"
	}
	req, err := http.NewRequestWithContext(ctx, "POST", c.Endpoint.URL+path, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Host = c.Endpoint.Host
	req.Header.Set("Authorization", "Bearer "+c.Endpoint.Token)
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Accept", "application/json, text/event-stream")
	req.Header.Set("MCP-Protocol-Version", "2025-06-18")
	client := &http.Client{Timeout: 20 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	raw, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if c.Observe != nil {
		c.Observe(resp.Header, raw)
	}
	if c.DropResponse {
		return io.ErrUnexpectedEOF
	}
	if c.Protocol == "mcp" {
		var envelope struct {
			Result struct {
				StructuredContent json.RawMessage `json:"structuredContent"`
			} `json:"result"`
			Error json.RawMessage `json:"error"`
		}
		if json.Unmarshal(raw, &envelope) != nil || envelope.Result.StructuredContent == nil {
			return contracts.ErrCorrupt
		}
		raw = envelope.Result.StructuredContent
	}
	var problem struct {
		Error *contracts.Error `json:"error"`
	}
	if json.Unmarshal(raw, &problem) == nil && problem.Error != nil {
		return problem.Error
	}
	if resp.StatusCode != 200 {
		return contracts.ErrUnavailable
	}
	if output != nil {
		return json.Unmarshal(raw, output)
	}
	return nil
}
