package phonefixture

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net"
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

func (g *Gateway) Agent(level string) (string, string, error) {
	raw, err := g.Command("agent", "create", "--name", "[ab-test] "+level, "--level", level, "--scope", "iphone:write")
	var result struct {
		Token string `json:"token"`
		Agent struct {
			ID string `json:"id"`
		} `json:"agent"`
	}
	if err == nil {
		err = json.Unmarshal([]byte(raw), &result)
	}
	return result.Token, result.Agent.ID, err
}
func (g *Gateway) Expire(source, id string) error     { return g.Lab("phone-expired", source, id, nil) }
func (g *Gateway) Record(id string, output any) error { return g.Lab("phone-command", id, "", output) }
func (g *Gateway) Lab(action, source, id string, output any) error {
	transport := &http.Transport{DialContext: func(ctx context.Context, _, _ string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "unix", filepath.Join(g.Home, "run", "blobs-lab.sock"))
	}}
	defer transport.CloseIdleConnections()
	raw, _ := json.Marshal(map[string]string{"action": action, "owner": source, "pin": id})
	response, err := (&http.Client{Transport: transport, Timeout: 5 * time.Second}).Post("http://lab/", "application/json", bytes.NewReader(raw))
	if err != nil {
		return err
	}
	defer response.Body.Close()
	if response.StatusCode != 200 {
		return fmt.Errorf("expiry fixture HTTP %d", response.StatusCode)
	}
	if output != nil {
		return json.NewDecoder(response.Body).Decode(output)
	}
	return nil
}
