package scenarios

import (
	"context"
	"net/url"
	"os/exec"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/remote"
)

func relayIngressReady(ctx context.Context, status remote.Status, u *url.URL, ip string, e *Evidence, add func(string, bool)) bool {
	started := time.Now()
	bounded, cancel := context.WithTimeout(ctx, 120*time.Second)
	defer cancel()
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	port := u.Port()
	if port == "" {
		port = "443"
	}
	ready := false
	for {
		attempt, stop := context.WithTimeout(bounded, 12*time.Second)
		cmd := exec.CommandContext(attempt, "ssh", "ubuntu", "curl", "--noproxy", "'*'", "--fail", "--silent", "--show-error", "--max-time", "10", "--resolve", u.Hostname()+":"+port+":"+ip, status.URL+"/.well-known/oauth-authorization-server")
		output, err := cmd.Output()
		stop()
		if err == nil && len(output) > 0 {
			ready = true
			break
		}
		select {
		case <-bounded.Done():
		case <-ticker.C:
			continue
		}
		break
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["public_ingress_ready_ms"] = float64(time.Since(started)) / float64(time.Millisecond)
	add("public_ingress_ready_within_120_seconds", ready)
	if !ready {
		relayIngressStatus(e.RunID)
		e.Assertions[len(e.Assertions)-1].Detail = "private redacted funnel-status.txt and serve-status.json captured while mapping was enabled"
	}
	return ready
}
