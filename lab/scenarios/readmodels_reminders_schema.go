package scenarios

import (
	"context"
	"encoding/json"
	"net/http"
	"reflect"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func reminderReadModelSchema(ctx context.Context, w *world.World, operation string, add func(string, bool)) {
	request, err := http.NewRequestWithContext(ctx, http.MethodGet, w.Gateway.URL+"/v1/openapi.json", nil)
	if err != nil {
		add("reminders_openapi_request", false)
		return
	}
	request.Host = w.Gateway.Host
	request.Header.Set("Authorization", "Bearer "+w.Gateway.Token)
	response, err := (&http.Client{Timeout: 2 * time.Second}).Do(request)
	add("reminders_openapi_available", err == nil)
	if err != nil {
		return
	}
	defer response.Body.Close()
	var document struct{ Paths map[string]json.RawMessage }
	err = json.NewDecoder(response.Body).Decode(&document)
	path := document.Paths["/v1/op/"+operation]
	valid := response.StatusCode == http.StatusOK && err == nil && len(path) > 0
	name := operation + "_output_schema"
	if operation == "reminders.items" {
		var shape map[string]any
		json.Unmarshal(path, &shape)
		for _, key := range []string{"post", "responses", "200", "content", "application/json", "schema"} {
			shape, _ = shape[key].(map[string]any)
		}
		definitions, _ := shape["$defs"].(map[string]any)
		item, _ := definitions["contracts.ReminderItem"].(map[string]any)
		properties, _ := item["properties"].(map[string]any)
		subtasks, _ := properties["subtasks"].(map[string]any)
		child, _ := subtasks["items"].(map[string]any)
		reference, _ := child["$ref"].(string)
		id, _ := shape["$id"].(string)
		valid = valid && id != "" && reference == "#/$defs/contracts.ReminderItem" && definitions[strings.TrimPrefix(reference, "#/$defs/")] != nil
		name += "_with_recursive_subtasks"
	}
	add(name, valid)
}

func reminderReadModelProtocolParity(ctx context.Context, w *world.World, operation string, query map[string]any, add func(string, bool)) {
	var rest, mcp map[string]any
	first := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, operation, query, &rest)
	var responseBody []byte
	second := (scripted.Client{Endpoint: w.Gateway, Protocol: "mcp", Observe: func(_ http.Header, body []byte) { responseBody = body }}).Call(ctx, operation, query, &mcp)
	add(operation+"_mcp_text_matches_structured_json", second == nil && reminderMCPTextEqual(responseBody))
	delete(rest, "cursor")
	delete(mcp, "cursor")
	add(operation+"_rest_mcp_structured_result_exact", first == nil && second == nil && reflect.DeepEqual(rest, mcp))
}

func reminderMCPTextEqual(body []byte) bool {
	var response struct {
		Result struct {
			StructuredContent any `json:"structuredContent"`
			Content           []struct {
				Text string `json:"text"`
			} `json:"content"`
		} `json:"result"`
	}
	if json.Unmarshal(body, &response) != nil || len(response.Result.Content) == 0 {
		return false
	}
	var text any
	return json.Unmarshal([]byte(response.Result.Content[0].Text), &text) == nil && reflect.DeepEqual(text, response.Result.StructuredContent)
}
