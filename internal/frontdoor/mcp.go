package frontdoor

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func mcpHandler(deps Deps, descriptors []contracts.OperationDescriptor) http.Handler {
	server := mcp.NewServer(&mcp.Implementation{Name: "apple-bridge", Version: "1"}, nil)
	for _, d := range descriptors {
		server.AddTool(&mcp.Tool{Name: strings.ReplaceAll(d.Name, ".", "_"), Description: fmt.Sprintf("%s (operation version %d)", d.Description, d.Version), InputSchema: contracts.OperationSchema(d)}, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
			p, ok := ctx.Value(principalKey{}).(trust.Principal)
			if !ok {
				return nil, contracts.ErrUnauthorized
			}
			output, milliseconds, err := invokeTimed(ctx, deps, p, d, req.Params.Arguments)
			if err != nil {
				typed := contracts.ErrUnavailable
				var problem *contracts.Error
				if errors.As(err, &problem) {
					typed = problem
				}
				output = map[string]any{"error": typed}
			}
			encoded, marshalErr := json.Marshal(output)
			if marshalErr != nil {
				return nil, marshalErr
			}
			var meta mcp.Meta
			if d.ReadModel {
				meta = mcp.Meta{"bridge/read_model_ms": milliseconds}
			}
			return &mcp.CallToolResult{Meta: meta, IsError: err != nil, StructuredContent: json.RawMessage(encoded), Content: []mcp.Content{&mcp.TextContent{Text: string(encoded)}}}, nil
		})
	}
	handler := mcp.NewStreamableHTTPHandler(func(*http.Request) *mcp.Server { return server }, &mcp.StreamableHTTPOptions{Stateless: true, JSONResponse: deps.PublicHost == nil, DisableLocalhostProtection: deps.PublicHost != nil})
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method == http.MethodPost {
			original := r.Body
			defer original.Close()
			body, err := io.ReadAll(io.LimitReader(original, 1024*1024))
			if err != nil {
				respond(w, nil, contracts.ErrInvalid)
				return
			}
			r.Body = io.NopCloser(io.MultiReader(bytes.NewReader(body), original))
			var call struct {
				ID     json.RawMessage `json:"id"`
				Method string          `json:"method"`
				Params struct {
					Name string `json:"name"`
				} `json:"params"`
			}
			if json.Unmarshal(body, &call) == nil && call.Method == "tools/call" && strings.HasPrefix(call.Params.Name, "usage_") {
				w.Header().Set("Content-Type", "application/json")
				json.NewEncoder(w).Encode(map[string]any{"jsonrpc": "2.0", "id": call.ID, "result": map[string]any{"isError": true, "structuredContent": map[string]any{"error": contracts.ErrUnsupported}, "content": []map[string]string{{"type": "text", "text": "operation unsupported"}}}})
				return
			}
		}
		handler.ServeHTTP(w, r)
	})
}
func openAPI(descriptors []contracts.OperationDescriptor) any {
	paths := map[string]any{}
	uploadSchema := map[string]any{"type": "object", "properties": map[string]any{"upload_id": map[string]any{"type": "string"}}}
	uploadResponse := map[string]any{"description": "Private upload reference", "content": map[string]any{"application/json": map[string]any{"schema": uploadSchema}}}
	uploadBody := map[string]any{"required": true, "content": map[string]any{"application/octet-stream": map[string]any{"schema": map[string]any{"type": "string", "format": "binary"}}}}
	paths["/v1/blobs"] = map[string]any{"post": map[string]any{"operationId": "stageMessageUpload", "description": "Stage bytes for messages.send and use upload_id in files.", "requestBody": uploadBody, "responses": map[string]any{"200": uploadResponse}}}

	for _, d := range descriptors {
		paths["/v1/op/"+d.Name] = map[string]any{"post": map[string]any{"operationId": d.Name, "description": d.Description, "requestBody": map[string]any{"required": true, "content": map[string]any{"application/json": map[string]any{"schema": contracts.OperationSchema(d)}}}, "responses": map[string]any{"200": map[string]any{"description": "Operation result", "content": map[string]any{"application/json": map[string]any{"schema": contracts.JSONSchema(d.Output)}}}}}}
	}
	return map[string]any{"openapi": "3.1.0", "info": map[string]string{"title": "Bridge", "version": "1"}, "paths": paths}
}
