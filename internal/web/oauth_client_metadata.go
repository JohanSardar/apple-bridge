package web

import (
	"context"
	"encoding/json"
	"io"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type ClientMetadataFetcher struct {
	Resolve     func(context.Context, string) ([]net.IPAddr, error)
	Transport   http.RoundTripper
	DialContext func(context.Context, string, string) (net.Conn, error)
	mu          sync.Mutex
	cache       map[string]clientMetadataEntry
}
type clientMetadataEntry struct {
	client  trust.OAuthClient
	expires time.Time
}

func metadataURL(raw string) (*url.URL, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "https" || u.Hostname() == "" || u.User != nil || strings.Contains(raw, "#") || u.Path == "" || u.Path == "/" || len(raw) > 2048 {
		return nil, contracts.ErrInvalid
	}
	for _, segment := range strings.Split(u.Path, "/") {
		if segment == "." || segment == ".." {
			return nil, contracts.ErrInvalid
		}
	}
	return u, nil
}
func publicMetadataIP(ip net.IP) bool {
	a, ok := netip.AddrFromSlice(ip)
	if !ok {
		return false
	}
	a = a.Unmap()
	if !a.IsGlobalUnicast() || a.IsPrivate() || a.IsLoopback() || a.IsLinkLocalUnicast() {
		return false
	}
	for _, prefix := range []string{"0.0.0.0/8", "100.64.0.0/10", "192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24", "240.0.0.0/4", "2001::/32", "192.0.0.0/24", "198.18.0.0/15", "2001:db8::/32", "64:ff9b::/96", "64:ff9b:1::/48", "2002::/16"} {
		if netip.MustParsePrefix(prefix).Contains(a) {
			return false
		}
	}
	return true
}
func (f *ClientMetadataFetcher) addresses(ctx context.Context, host string) ([]net.IPAddr, error) {
	resolve := f.Resolve
	if resolve == nil {
		resolve = net.DefaultResolver.LookupIPAddr
	}
	ips, err := resolve(ctx, host)
	if err != nil || len(ips) == 0 {
		return nil, contracts.ErrInvalid
	}
	for _, ip := range ips {
		if !publicMetadataIP(ip.IP) {
			return nil, contracts.ErrInvalid
		}
	}
	return ips, nil
}
func (f *ClientMetadataFetcher) Fetch(ctx context.Context, id, redirect string) (trust.OAuthClient, error) {
	empty := trust.OAuthClient{}
	u, err := metadataURL(id)
	if err != nil {
		return empty, err
	}
	f.mu.Lock()
	entry, ok := f.cache[id]
	f.mu.Unlock()
	if ok && time.Now().Before(entry.expires) {
		if !slices.Contains(entry.client.RedirectURIs, redirect) {
			return empty, contracts.ErrInvalid
		}
		return entry.client, nil
	}
	ctx, cancel := context.WithTimeout(ctx, 3*time.Second)
	defer cancel()
	if _, err = f.addresses(ctx, u.Hostname()); err != nil {
		return empty, err
	}
	transport := f.Transport
	if transport == nil {
		t := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
			host, port, err := net.SplitHostPort(address)
			if err != nil {
				return nil, err
			}
			ips, err := f.addresses(ctx, host)
			if err != nil {
				return nil, err
			}
			dial := f.DialContext
			if dial == nil {
				dial = (&net.Dialer{}).DialContext
			}
			return dial(ctx, network, net.JoinHostPort(ips[0].IP.String(), port))
		}}
		defer t.CloseIdleConnections()
		transport = t
	}
	client := &http.Client{Transport: transport, Timeout: 3 * time.Second, CheckRedirect: func(r *http.Request, via []*http.Request) error {
		if len(via) >= 5 {
			return contracts.ErrInvalid
		}
		if _, err := metadataURL(r.URL.String()); err != nil {
			return err
		}
		_, err := f.addresses(r.Context(), r.URL.Hostname())
		return err
	}}
	req, err := http.NewRequestWithContext(ctx, "GET", id, nil)
	if err != nil {
		return empty, err
	}
	resp, err := client.Do(req)
	if err != nil {
		return empty, err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return empty, contracts.ErrInvalid
	}
	raw, err := io.ReadAll(io.LimitReader(resp.Body, 5121))
	if err != nil || len(raw) > 5120 {
		return empty, contracts.ErrInvalid
	}
	var c trust.OAuthClient
	if json.Unmarshal(raw, &c) != nil || c.ID != id || len(c.RedirectURIs) == 0 || !slices.Contains(c.RedirectURIs, redirect) || (c.TokenEndpointAuthMethod != "" && c.TokenEndpointAuthMethod != "none") {
		return empty, contracts.ErrInvalid
	}
	for _, redirect := range c.RedirectURIs {
		if !trust.ValidOAuthRedirect(redirect) {
			return empty, contracts.ErrInvalid
		}
	}
	ttl := time.Duration(0)
	for _, part := range strings.Split(strings.Join(resp.Header.Values("Cache-Control"), ","), ",") {
		key, value, _ := strings.Cut(strings.TrimSpace(part), "=")
		key = strings.ToLower(key)
		if key == "no-store" || key == "no-cache" {
			ttl = 0
			break
		}
		if key == "max-age" {
			n, e := strconv.ParseUint(strings.Trim(value, `"`), 10, 32)
			if e == nil {
				ttl = time.Duration(n) * time.Second
			}
		}
	}
	if age, err := strconv.ParseUint(resp.Header.Get("Age"), 10, 32); err == nil {
		ttl -= time.Duration(age) * time.Second
	}
	if ttl > 0 {
		f.mu.Lock()
		if f.cache == nil {
			f.cache = map[string]clientMetadataEntry{}
		}
		if len(f.cache) >= 1000 {
			clear(f.cache)
		}
		f.cache[id] = clientMetadataEntry{c, time.Now().Add(ttl)}
		f.mu.Unlock()
	}
	return c, nil
}
