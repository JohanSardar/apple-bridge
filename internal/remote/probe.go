package remote

import (
	"bufio"
	"context"
	"fmt"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"strings"
	"time"
)

const streamCheckPath = "/.well-known/apple-bridge-remote-check"

func publicProbe(ctx context.Context, origin string) (time.Time, error) {
	u, err := url.Parse(origin)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" {
		return time.Time{}, fmt.Errorf("invalid public origin")
	}
	resolver := &net.Resolver{PreferGo: true, Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
		return (&net.Dialer{}).DialContext(ctx, "udp", "1.1.1.1:53")
	}}
	lookup, cancel := context.WithTimeout(ctx, 10*time.Second)
	addresses, err := resolver.LookupNetIP(lookup, "ip4", u.Hostname())
	cancel()
	if err != nil {
		return time.Time{}, err
	}
	port := u.Port()
	if port == "" {
		port = "443"
	}
	target := ""
	for _, ip := range addresses {
		if publicAddress(ip) {
			target = net.JoinHostPort(ip.String(), port)
			break
		}
	}
	if target == "" {
		return time.Time{}, fmt.Errorf("no public Funnel address")
	}
	transport := &http.Transport{DialContext: func(ctx context.Context, network, addr string) (net.Conn, error) {
		return (&net.Dialer{Timeout: 10 * time.Second}).DialContext(ctx, "tcp", target)
	}, ResponseHeaderTimeout: 10 * time.Second}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, CheckRedirect: func(*http.Request, []*http.Request) error { return http.ErrUseLastResponse }}
	return ProbeHTTP(ctx, client, origin, 60*time.Second)
}

func ProbeHTTP(ctx context.Context, client *http.Client, origin string, duration time.Duration) (time.Time, error) {
	var expiry time.Time
	for _, check := range []struct {
		path string
		code int
	}{{"/.well-known/oauth-authorization-server", 200}, {"/mcp", 401}, {"/", 200}, {streamCheckPath, 200}} {
		bounded, cancel := context.WithTimeout(ctx, duration+15*time.Second)
		req, err := http.NewRequestWithContext(bounded, "GET", origin+check.path, nil)
		if err != nil {
			cancel()
			return expiry, err
		}
		if check.path == streamCheckPath {
			req.Header.Set("Accept", "text/event-stream")
		}
		response, err := client.Do(req)
		if check.path == "/.well-known/oauth-authorization-server" {
			for err != nil && bounded.Err() == nil {
				select {
				case <-bounded.Done():
				case <-time.After(100 * time.Millisecond):
				}
				response, err = client.Do(req)
			}
		}
		if err != nil {
			cancel()
			return expiry, err
		}
		if response.TLS != nil && len(response.TLS.PeerCertificates) > 0 {
			expiry = response.TLS.PeerCertificates[0].NotAfter
		}
		err = validateProbe(bounded, response, check.path, check.code, duration)
		response.Body.Close()
		cancel()
		if err != nil {
			return expiry, err
		}
	}
	return expiry, nil
}

func validateProbe(ctx context.Context, response *http.Response, path string, code int, duration time.Duration) error {
	if response.StatusCode != code {
		return fmt.Errorf("public probe %s: HTTP %d, want %d", path, response.StatusCode, code)
	}
	if path == "/mcp" && response.Header.Get("WWW-Authenticate") == "" {
		return fmt.Errorf("public MCP challenge missing")
	}
	if path != streamCheckPath {
		return nil
	}
	if !strings.HasPrefix(response.Header.Get("Content-Type"), "text/event-stream") {
		return fmt.Errorf("public SSE content type missing")
	}
	start := time.Now()
	scanner := bufio.NewScanner(response.Body)
	for scanner.Scan() {
		if ctx.Err() != nil {
			return ctx.Err()
		}
		if scanner.Text() == ": bridge remote check" && time.Since(start) >= duration {
			return nil
		}
	}
	return fmt.Errorf("public SSE ended before %s: %v", duration, scanner.Err())
}

func publicAddress(address netip.Addr) bool {
	address = address.Unmap()
	return address.IsGlobalUnicast() && !address.IsPrivate() && !netip.MustParsePrefix("100.64.0.0/10").Contains(address)
}
