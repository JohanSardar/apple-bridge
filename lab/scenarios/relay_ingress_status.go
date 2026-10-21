package scenarios

import (
	"context"
	"encoding/json"
	"net"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"time"
)

func relayIngressStatus(runID string) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	dir := filepath.Join("lab/evidence/private/relay", runID)
	os.MkdirAll(dir, 0700)
	raw, _ := exec.CommandContext(ctx, "/opt/homebrew/bin/tailscale", "funnel", "status").CombinedOutput()
	urls := regexp.MustCompile(`https?://[^\s]+`)
	redacted := urls.ReplaceAllStringFunc(string(raw), func(value string) string {
		u, err := url.Parse(value)
		if err != nil {
			return "<redacted-url>"
		}
		port := ""
		if u.Port() != "" {
			port = ":" + u.Port()
		}
		return u.Scheme + "://<redacted-host>" + port
	})
	paths := regexp.MustCompile(`(?m)(\|--\s+)\S+`)
	redacted = paths.ReplaceAllString(redacted, "${1}<redacted-path>")
	os.WriteFile(filepath.Join(dir, "funnel-status.txt"), []byte(redacted), 0600)
	raw, _ = exec.CommandContext(ctx, "/opt/homebrew/bin/tailscale", "serve", "status", "--json").Output()
	var value any
	if json.Unmarshal(raw, &value) == nil {
		raw, _ = json.MarshalIndent(relayRedactStatus(value), "", "  ")
	} else {
		raw = []byte(`{"error":"serve status unavailable"}`)
	}
	os.WriteFile(filepath.Join(dir, "serve-status.json"), raw, 0600)
}
func relayRedactStatus(value any) any {
	switch item := value.(type) {
	case string:
		return "<redacted>"
	case map[string]any:
		out := map[string]any{}
		for key, value := range item {
			if _, port, err := net.SplitHostPort(key); err == nil {
				key = "<redacted-host>:" + port
			}
			if len(key) > 0 && key[0] == '/' && key != "/" {
				key = "<redacted-path>"
			}
			out[key] = relayRedactStatus(value)
		}
		return out
	case []any:
		for index := range item {
			item[index] = relayRedactStatus(item[index])
		}
	}
	return value
}
