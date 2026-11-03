package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
)

func macSchemaOperations() []contracts.OperationDescriptor {
	var operations []contracts.OperationDescriptor
	for _, name := range []string{"mac.set_volume", "mac.set_mute", "mac.set_appearance", "mac.launch_app", "mac.activate_app", "mac.quit_app", "mac.keep_awake", "mac.clipboard_write", "mac.notify", "mac.open_settings", "mac.set_wallpaper", "mac.lock_screen", "mac.sleep_display", "mac.restart", "mac.shutdown", "mac.iwork_export"} {
		operations = append(operations, contracts.OperationDescriptor{Name: name, Kind: contracts.CommandOp})
	}
	for _, name := range []string{"mac.clipboard_read", "mac.network_check", "mac.bluetooth_devices", "mac.time_machine_status", "mac.recognize_song"} {
		operations = append(operations, contracts.OperationDescriptor{Name: name, Kind: contracts.QueryOp})
	}
	return operations
}

func macSchemaValue(value any) any {
	switch item := value.(type) {
	case map[string]any:
		for key, nested := range item {
			if key == "required" {
				if values, ok := nested.([]any); ok {
					strings := make([]string, len(values))
					valid := true
					for i, value := range values {
						var ok bool
						strings[i], ok = value.(string)
						valid = valid && ok
					}
					if valid {
						item[key] = strings
						continue
					}
				}
			}
			item[key] = macSchemaValue(nested)
		}
	case []any:
		for i, nested := range item {
			item[i] = macSchemaValue(nested)
		}
	case json.Number:
		if number, err := item.Int64(); err == nil {
			return int(number)
		}
	}
	return value
}

func macBinarySchemas(ctx context.Context, w *world.World) (map[string]map[string]any, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", w.Gateway.URL+"/v1/openapi.json", nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+w.Gateway.Token)
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		return nil, err
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		return nil, fmt.Errorf("OpenAPI status %d", response.StatusCode)
	}
	var document struct {
		Paths map[string]struct {
			Post struct {
				RequestBody struct {
					Content map[string]struct {
						Schema map[string]any `json:"schema"`
					} `json:"content"`
				} `json:"requestBody"`
			} `json:"post"`
		} `json:"paths"`
	}
	decoder := json.NewDecoder(io.LimitReader(response.Body, 16*1024*1024))
	decoder.UseNumber()
	if err = decoder.Decode(&document); err != nil {
		return nil, err
	}
	schemas := map[string]map[string]any{}
	for _, op := range macSchemaOperations() {
		schema := document.Paths["/v1/op/"+op.Name].Post.RequestBody.Content["application/json"].Schema
		if schema == nil {
			return nil, fmt.Errorf("missing schema %s", op.Name)
		}
		schemas[op.Name] = macSchemaValue(schema).(map[string]any)
	}
	return schemas, nil
}
