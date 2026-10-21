package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/remote"
	"net"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
)

func remoteProxyChecks(ctx context.Context, home string, status remote.Status, u *url.URL, add func(string, bool)) {
	var mapping struct {
		Target string `json:"target"`
	}
	leaseBytes, _ := os.ReadFile(filepath.Join(home, "remote", "lease.json"))
	json.Unmarshal(leaseBytes, &mapping)
	proxyTarget, _ := url.Parse(mapping.Target)
	proxyRequest := func(path, forwardedHost, source string) int {
		req, _ := http.NewRequestWithContext(ctx, "POST", proxyTarget.Scheme+"://"+proxyTarget.Host+path, strings.NewReader("{}"))
		req.Host = u.Host
		req.Header.Set("Origin", status.URL)
		req.Header.Set("X-Forwarded-Host", forwardedHost)
		req.Header.Set("X-Forwarded-Proto", "https")
		req.Header.Set("X-Forwarded-For", source)
		response, err := http.DefaultClient.Do(req)
		if err != nil {
			return 0
		}
		defer response.Body.Close()
		return response.StatusCode
	}
	add("direct_loopback_forwarding_spoof_rejected", proxyRequest("/web/v1/login/options", u.Host, "192.0.2.40") == 403)
	add("proxy_unconfigured_forwarded_host_rejected", proxyRequest(proxyTarget.Path+"/web/v1/login/options", "attacker.invalid", "192.0.2.40") == 403)
	ownIPs := true
	for i := 0; i < 10; i++ {
		ownIPs = ownIPs && proxyRequest(proxyTarget.Path+"/web/v1/login/options", u.Host, "192.0.2.40") == 200
	}
	add("system_original_client_ip_rate_bucket", ownIPs && proxyRequest(proxyTarget.Path+"/web/v1/login/options", u.Host, "192.0.2.40") == 429)
	add("system_distinct_client_ip_not_loopback_bucket", proxyRequest(proxyTarget.Path+"/web/v1/login/options", u.Host, "192.0.2.41") == 200)

}
func remoteMappingChecks(ctx context.Context, before []byte, u *url.URL, add func(string, bool)) {
	during, err := exec.CommandContext(ctx, "/opt/homebrew/bin/tailscale", "serve", "status", "--json").Output()
	var original, changed map[string]any
	json.Unmarshal(before, &original)
	json.Unmarshal(during, &changed)
	hp := u.Host
	if u.Port() == "" {
		hp += ":443"
	}
	port := u.Port()
	if port == "" {
		port = "443"
	}
	ownOnly := err == nil
	for _, key := range []string{"TCP", "Web", "AllowFunnel"} {
		section, ok := changed[key].(map[string]any)
		if !ok {
			ownOnly = false
			continue
		}
		index := hp
		if key == "TCP" {
			index = port
		}
		_, exists := section[index]
		ownOnly = ownOnly && exists
		delete(section, index)
		if len(section) == 0 && original[key] == nil {
			delete(changed, key)
		}
	}
	add("existing_handlers_preserved_exactly_while_on", ownOnly && reflect.DeepEqual(original, changed))
}
func remotePublicChecks(ctx context.Context, status remote.Status, u *url.URL, add func(string, bool)) string {
	dns, err := exec.CommandContext(ctx, "ssh", "ubuntu", "dig", "@1.1.1.1", "+short", "A", u.Hostname()).Output()
	ip := ""
	for _, line := range strings.Fields(string(dns)) {
		if parsed := net.ParseIP(line); parsed != nil && !parsed.IsPrivate() && !parsed.IsLoopback() {
			ip = line
			break
		}
	}
	add("ubuntu_public_dns", err == nil && ip != "")
	if ip == "" {
		return ""
	}
	port := u.Port()
	if port == "" {
		port = "443"
	}
	resolve := u.Hostname() + ":" + port + ":" + ip
	for _, path := range []string{"/", "/v1/health"} {
		cmd := exec.CommandContext(ctx, "ssh", "ubuntu", "curl", "--noproxy", "'*'", "--fail", "--silent", "--show-error", "--max-time", "10", "--resolve", resolve, status.URL+path)
		output, err := cmd.Output()
		add("ubuntu_curl_public_tls"+path, err == nil && len(output) > 0)
	}
	return ip
}
