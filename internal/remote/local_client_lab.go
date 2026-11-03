//go:build lab

package remote

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"net"
	"net/http"
	"os"
	"tailscale.com/client/local"
	"time"
)

func installedLocalClient() *local.Client {
	socket := os.Getenv("APPLE_BRIDGE_TAILSCALE_SOCKET")
	return &local.Client{Socket: socket, UseSocketOnly: socket != "", OmitAuth: socket != ""}
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
	if root := os.Getenv("APPLE_BRIDGE_LAB_TLS_ROOT"); root != "" {
		raw, err := os.ReadFile(root)
		if err != nil {
			return time.Time{}, err
		}
		roots := x509.NewCertPool()
		if !roots.AppendCertsFromPEM(raw) {
			return time.Time{}, fmt.Errorf("invalid lab TLS root")
		}
		transport.TLSClientConfig = &tls.Config{MinVersion: tls.VersionTLS12, RootCAs: roots}
	}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	duration := 60 * time.Second
	if configured := os.Getenv("APPLE_BRIDGE_LAB_PROBE_DURATION"); configured != "" {
		parsed, err := time.ParseDuration(configured)
		if err != nil || parsed < time.Second || parsed > duration {
			return time.Time{}, fmt.Errorf("invalid lab probe duration")
		}
		duration = parsed
	}
	return ProbeHTTP(ctx, client, origin, duration)
}

func installedSetupFailure(err error) {
	if path := os.Getenv("APPLE_BRIDGE_LAB_REMOTE_DIAGNOSTICS"); path != "" && err != nil {
		_ = os.WriteFile(path, []byte(err.Error()), 0600)
	}
}
