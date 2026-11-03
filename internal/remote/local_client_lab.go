//go:build lab

package remote

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"os"
	"tailscale.com/client/local"
	"time"
)

func installedLocalClient() *local.Client {
	socket := os.Getenv("APPLE_BRIDGE_TAILSCALE_SOCKET")
	return &local.Client{Socket: socket, OmitAuth: socket != ""}
}
func installedProbe(ctx context.Context, origin string) (time.Time, error) {
	target := os.Getenv("APPLE_BRIDGE_REMOTE_PROBE_ADDRESS")
	if target == "" {
		return publicProbe(ctx, origin)
	}
	host, _, err := net.SplitHostPort(target)
	if err != nil || net.ParseIP(host) == nil || !net.ParseIP(host).IsLoopback() {
		return time.Time{}, fmt.Errorf("invalid lab probe address")
	}
	transport := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{Timeout: 10 * time.Second}).DialContext(ctx, "tcp", target)
	}, ResponseHeaderTimeout: 10 * time.Second}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	return ProbeHTTP(ctx, client, origin, 60*time.Second)
}
