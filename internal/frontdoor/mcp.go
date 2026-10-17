package frontdoor

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/modelcontextprotocol/go-sdk/mcp"
)

func mcpHandler(deps Deps, descriptors []contracts.OperationDescriptor) http.Handler {
	return mcp.NewStreamableHTTPHandler(func(r *http.Request) *mcp.Server {
		p := r.Context().Value(principalKey{}).(trust.Principal)
		server := mcp.NewServer(&mcp.Implementation{Name: "apple-bridge", Version: "1"}, nil)
		for _, d := range descriptors {
			server.AddTool(&mcp.Tool{Name: strings.ReplaceAll(d.Name, ".", "_"), Description: fmt.Sprintf("%s (operation version %d)", d.Description, d.Version), InputSchema: contracts.OperationSchema(d)}, func(ctx context.Context, req *mcp.CallToolRequest) (*mcp.CallToolResult, error) {
				output, err := deps.Operations.Invoke(ctx, p, d.Name, req.Params.Arguments)
				if err != nil {
					typed := contracts.ErrUnavailable
					var problem *contracts.Error
					if errors.As(err, &problem) {
						typed = problem
					}
					output = map[string]any{"error": typed}
				}
				b, marshalErr := json.Marshal(output)
				if marshalErr != nil {
					return nil, marshalErr
				}
				return &mcp.CallToolResult{IsError: err != nil, StructuredContent: output, Content: []mcp.Content{&mcp.TextContent{Text: string(b)}}}, nil
			})
		}
		return server
	}, &mcp.StreamableHTTPOptions{Stateless: true, JSONResponse: true})
}
func openAPI(descriptors []contracts.OperationDescriptor) any {
	paths := map[string]any{}
	for _, d := range descriptors {
		paths["/v1/op/"+d.Name] = map[string]any{"post": map[string]any{"operationId": d.Name, "description": d.Description, "requestBody": map[string]any{"required": true, "content": map[string]any{"application/json": map[string]any{"schema": contracts.OperationSchema(d)}}}, "responses": map[string]any{"200": map[string]any{"description": "Operation result", "content": map[string]any{"application/json": map[string]any{"schema": contracts.JSONSchema(d.Output)}}}}}}
	}
	return map[string]any{"openapi": "3.1.0", "info": map[string]string{"title": "Apple Bridge", "version": "1"}, "paths": paths}
}
