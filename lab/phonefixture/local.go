package phonefixture

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"time"
)

func (g *Gateway) Request(path string, input any, credential string, output any) (int, error) {
	var config struct {
		APIAddress string `json:"api_address"`
	}
	configBytes, err := os.ReadFile(filepath.Join(g.Home, "runtime.json"))
	if err != nil {
		return 0, err
	}
	if err = json.Unmarshal(configBytes, &config); err != nil {
		return 0, err
	}
	raw, err := json.Marshal(input)
	if err != nil {
		return 0, err
	}
	request, err := http.NewRequestWithContext(g.ctx, "POST", "http://"+config.APIAddress+path, bytes.NewReader(raw))
	if err != nil {
		return 0, err
	}
	request.Header.Set("Content-Type", "application/json")
	request.Header.Set("Authorization", "Bearer "+credential)
	client := &http.Client{Timeout: 15 * time.Second}
	response, err := client.Do(request)
	if err != nil {
		return 0, err
	}
	defer response.Body.Close()
	if output != nil && response.StatusCode == 200 {
		err = json.NewDecoder(response.Body).Decode(output)
	}
	return response.StatusCode, err
}

func (g *Gateway) Invoke(op string, input any, credential string, output any) error {
	status, err := g.Request("/v1/op/"+op, input, credential, output)
	if err != nil {
		return err
	}
	if status != 200 {
		return fmt.Errorf("%s: HTTP %d", op, status)
	}
	return nil
}
