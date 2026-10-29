package publichttp

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net"
	"net/http"
	"net/netip"
	"net/url"
	"time"
)

type Guard struct {
	Resolve     func(context.Context, string) ([]net.IPAddr, error)
	DialContext func(context.Context, string, string) (net.Conn, error)
}

func PublicIP(ip net.IP) bool {
	a, ok := netip.AddrFromSlice(ip)
	if !ok {
		return false
	}
	a = a.Unmap()
	if a.Is6() && !netip.MustParsePrefix("2000::/3").Contains(a) {
		return false
	}
	if !a.IsGlobalUnicast() || a.IsPrivate() || a.IsLoopback() || a.IsLinkLocalUnicast() {
		return false
	}
	for _, prefix := range []string{"0.0.0.0/8", "100.64.0.0/10", "192.0.2.0/24", "198.51.100.0/24", "203.0.113.0/24", "240.0.0.0/4", "2001::/23", "192.88.99.0/24", "3fff::/20", "192.0.0.0/24", "198.18.0.0/15", "2001:db8::/32", "64:ff9b::/96", "64:ff9b:1::/48", "2002::/16"} {
		if netip.MustParsePrefix(prefix).Contains(a) {
			return false
		}
	}
	return true
}
func (f *Guard) Addresses(ctx context.Context, host string) ([]net.IPAddr, error) {
	resolve := f.Resolve
	if resolve == nil {
		resolve = net.DefaultResolver.LookupIPAddr
	}
	ips, err := resolve(ctx, host)
	if err != nil || len(ips) == 0 {
		return nil, contracts.ErrInvalid
	}
	for _, ip := range ips {
		if !PublicIP(ip.IP) {
			return nil, contracts.ErrInvalid
		}
	}
	return ips, nil
}

func URL(raw string) (*url.URL, error) {
	u, e := url.Parse(raw)
	if e != nil || len(raw) > 2048 || u.Hostname() == "" || u.User != nil || (u.Scheme != "https" && u.Scheme != "http") {
		return nil, contracts.ErrInvalid
	}
	return u, nil
}
func (f *Guard) Client() (*http.Client, func()) {
	t := &http.Transport{DialContext: func(ctx context.Context, network, address string) (net.Conn, error) {
		host, port, e := net.SplitHostPort(address)
		if e != nil {
			return nil, e
		}
		ips, e := f.Addresses(ctx, host)
		if e != nil {
			return nil, e
		}
		dial := f.DialContext
		if dial == nil {
			dial = (&net.Dialer{}).DialContext
		}
		return dial(ctx, network, net.JoinHostPort(ips[0].IP.String(), port))
	}, MaxResponseHeaderBytes: 32768, ResponseHeaderTimeout: 3 * time.Second, DisableKeepAlives: true}
	c := &http.Client{Transport: t, Timeout: 3 * time.Second, CheckRedirect: func(r *http.Request, via []*http.Request) error {
		if len(via) >= 5 {
			return contracts.ErrInvalid
		}
		if _, e := URL(r.URL.String()); e != nil {
			return e
		}
		_, e := f.Addresses(r.Context(), r.URL.Hostname())
		return e
	}}
	return c, t.CloseIdleConnections
}
