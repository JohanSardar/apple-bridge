package remote

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strconv"
	"strings"
	"tailscale.com/client/local"
	"tailscale.com/ipn/ipnstate"
)

type object map[string]json.RawMessage

type lease struct {
	Host   string          `json:"host"`
	Port   uint16          `json:"port"`
	Target string          `json:"target"`
	Absent map[string]bool `json:"absent"`
}

func raw(value any) json.RawMessage { data, _ := json.Marshal(value); return data }
func section(config object, key string) (object, error) {
	result := object{}
	if value := config[key]; len(value) != 0 && string(value) != "null" {
		if err := json.Unmarshal(value, &result); err != nil {
			return nil, err
		}
	}
	return result, nil
}

func getServe(ctx context.Context, client *local.Client) (object, string, error) {
	req, _ := http.NewRequestWithContext(ctx, "GET", "http://local-tailscaled.sock/localapi/v0/serve-config", nil)
	resp, err := client.DoLocalRequest(req)
	if err != nil {
		return nil, "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return nil, "", fmt.Errorf("get serve config: HTTP %d", resp.StatusCode)
	}
	var config object
	if err = json.NewDecoder(resp.Body).Decode(&config); err != nil {
		return nil, "", err
	}
	if config == nil {
		config = object{}
	}
	etag := resp.Header.Get("ETag")
	if etag == "" {
		return nil, "", fmt.Errorf("serve config has no ETag; refusing unsafe update")
	}
	return config, etag, nil
}

func setServe(ctx context.Context, client *local.Client, config object, etag string) (bool, error) {
	req, _ := http.NewRequestWithContext(ctx, "POST", "http://local-tailscaled.sock/localapi/v0/serve-config", bytes.NewReader(raw(config)))
	req.Header.Set("If-Match", etag)
	req.Header.Set("Content-Type", "application/json")
	resp, err := client.DoLocalRequest(req)
	if err != nil {
		return false, err
	}
	defer resp.Body.Close()
	io.Copy(io.Discard, resp.Body)
	if resp.StatusCode == http.StatusPreconditionFailed {
		return false, nil
	}
	if resp.StatusCode != 200 {
		return false, fmt.Errorf("set serve config: HTTP %d", resp.StatusCode)
	}
	return true, nil
}

func systemStatus(ctx context.Context, client *local.Client) (*ipnstate.Status, error) {
	status, err := client.StatusWithoutPeers(ctx)
	if err != nil {
		return nil, err
	}
	if status.BackendState != "Running" || status.Self == nil || status.Self.DNSName == "" {
		return nil, fmt.Errorf("system Tailscale is not running and logged in")
	}
	return status, nil
}

func portUsed(config object, port uint16) bool {
	p := strconv.Itoa(int(port))
	for _, key := range []string{"TCP", "Web", "AllowFunnel"} {
		values, err := section(config, key)
		if err != nil {
			return true
		}
		for k := range values {
			if k == p || strings.HasSuffix(k, ":"+p) {
				return true
			}
		}
	}
	for _, key := range []string{"Foreground", "Services"} {
		values, err := section(config, key)
		if err != nil {
			return true
		}
		for _, value := range values {
			var nested object
			if json.Unmarshal(value, &nested) != nil || portUsed(nested, port) {
				return true
			}
		}
	}
	return false
}
