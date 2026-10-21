package remote

import (
	"context"
	"encoding/json"
	"fmt"
	"net"
	"strconv"
	"strings"
	"tailscale.com/client/local"
	"tailscale.com/ipn"
	"time"
)

func addSystem(ctx context.Context, home string, client *local.Client, target string) (lease, error) {
	status, err := systemStatus(ctx, client)
	if err != nil {
		return lease{}, err
	}
	if err = ipn.NodeCanFunnel(status.Self); err != nil {
		return lease{}, fmt.Errorf("%s", enablement(ctx, client, err))
	}
	for attempt := 0; attempt < 5; attempt++ {
		config, etag, err := getServe(ctx, client)
		if err != nil {
			return lease{}, err
		}
		var port uint16
		for _, candidate := range []uint16{443, 8443, 10000} {
			if !portUsed(config, candidate) && ipn.CheckFunnelAccess(candidate, status.Self) == nil {
				port = candidate
				break
			}
		}
		if port == 0 {
			return lease{}, fmt.Errorf("no free Funnel port among 443, 8443, 10000")
		}
		entry := lease{Host: strings.TrimSuffix(status.Self.DNSName, "."), Port: port, Target: target, Absent: map[string]bool{}}
		hp := net.JoinHostPort(entry.Host, strconv.Itoa(int(port)))
		for key, value := range map[string]json.RawMessage{"TCP": raw(ipn.TCPPortHandler{HTTPS: true}), "Web": raw(ipn.WebServerConfig{Handlers: map[string]*ipn.HTTPHandler{"/": {Proxy: target}}}), "AllowFunnel": raw(true)} {
			values, err := section(config, key)
			if err != nil {
				return lease{}, err
			}
			_, exists := config[key]
			entry.Absent[key] = !exists
			index := hp
			if key == "TCP" {
				index = strconv.Itoa(int(port))
			}
			values[index] = value
			config[key] = raw(values)
		}
		if err = write(home, "lease", entry); err != nil {
			return lease{}, err
		}
		ok, err := setServe(ctx, client, config, etag)
		if err != nil {
			return lease{}, err
		}
		if ok {
			return entry, nil
		}
	}
	return lease{}, fmt.Errorf("serve config changed during five ETag attempts")
}

func enablement(ctx context.Context, client *local.Client, problem error) string {
	queryCtx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	response, err := client.QueryFeature(queryCtx, "funnel")
	if err == nil && response.URL != "" {
		return problem.Error() + "\n" + response.Text + "\n" + response.URL
	}
	return problem.Error() + "\nHTTPS: https://login.tailscale.com/admin/dns\nFunnel node attributes: https://login.tailscale.com/admin/acls"
}
