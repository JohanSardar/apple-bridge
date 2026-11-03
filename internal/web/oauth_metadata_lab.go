//go:build lab

package web

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"encoding/json"
	"net"
	"net/http"
	"net/url"
	"os"
	"time"
)

func labClientMetadata() *ClientMetadataFetcher {
	f := &ClientMetadataFetcher{}
	endpoint := os.Getenv("APPLE_BRIDGE_LAB_CIMD")
	if endpoint == "" {
		return f
	}
	read := func(ctx context.Context, path, host string, out any) error {
		req, err := http.NewRequestWithContext(ctx, "GET", endpoint+path+"?host="+url.QueryEscape(host), nil)
		if err != nil {
			return err
		}
		resp, err := (&http.Client{Timeout: time.Second}).Do(req)
		if err != nil {
			return err
		}
		defer resp.Body.Close()
		return json.NewDecoder(resp.Body).Decode(out)
	}
	f.Resolve = func(ctx context.Context, host string) ([]net.IPAddr, error) {
		var ips []string
		if err := read(ctx, "/resolve", host, &ips); err != nil {
			return nil, err
		}
		out := []net.IPAddr{}
		for _, ip := range ips {
			out = append(out, net.IPAddr{IP: net.ParseIP(ip)})
		}
		return out, nil
	}
	f.DialContext = func(ctx context.Context, network, address string) (net.Conn, error) {
		var target string
		if err := read(ctx, "/dial", address, &target); err != nil {
			return nil, err
		}
		return (&net.Dialer{}).DialContext(ctx, network, target)
	}
	f.Now = func() time.Time {
		var seconds int64
		if read(context.Background(), "/clock", "", &seconds) != nil {
			return time.Now()
		}
		return time.Unix(seconds, 0)
	}
	roots := x509.NewCertPool()
	pem, _ := os.ReadFile(os.Getenv("APPLE_BRIDGE_LAB_CIMD_CA"))
	roots.AppendCertsFromPEM(pem)
	f.TLSConfig = &tls.Config{RootCAs: roots, MinVersion: tls.VersionTLS12}
	return f
}
