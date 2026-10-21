package trust

import (
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"net"
	"net/url"
	"slices"
	"strings"
)

func validRedirect(raw string) bool {
	u, err := url.Parse(raw)
	if err != nil || u.Host == "" || u.Fragment != "" || u.User != nil || len(raw) > 2048 {
		return false
	}
	if u.Scheme == "https" {
		return true
	}
	ip := net.ParseIP(u.Hostname())
	return u.Scheme == "http" && (u.Hostname() == "localhost" || ip != nil && ip.IsLoopback())
}
func (a *Authority) RegisterOAuth(c OAuthClient) (OAuthClient, error) {
	if strings.TrimSpace(c.Name) == "" || len(c.Name) > 120 || len(c.RedirectURIs) == 0 || len(c.RedirectURIs) > 10 {
		return c, contracts.ErrInvalid
	}
	if c.TokenEndpointAuthMethod != "" && c.TokenEndpointAuthMethod != "none" {
		return c, contracts.ErrInvalid
	}
	for _, u := range c.RedirectURIs {
		if !validRedirect(u) {
			return c, contracts.ErrInvalid
		}
	}
	for _, g := range c.GrantTypes {
		if g != "authorization_code" && g != "refresh_token" {
			return c, contracts.ErrInvalid
		}
	}
	for _, g := range c.ResponseTypes {
		if g != "code" {
			return c, contracts.ErrInvalid
		}
	}
	c.ID = contracts.NewID("client")
	c.TokenEndpointAuthMethod = "none"
	if len(c.GrantTypes) == 0 {
		c.GrantTypes = []string{"authorization_code"}
	}
	c.ResponseTypes = []string{"code"}
	err := a.update(func(d *credentials) error {
		if len(d.OAuthClients) >= 1000 {
			return contracts.ErrUnavailable
		}
		d.OAuthClients = append(d.OAuthClients, c)
		return nil
	})
	return c, err
}
func (a *Authority) OAuthClient(id, redirect string) (OAuthClient, error) {
	d, err := a.load()
	if err != nil {
		return OAuthClient{}, err
	}
	for _, c := range d.OAuthClients {
		if c.ID == id && slices.Contains(c.RedirectURIs, redirect) {
			return c, nil
		}
	}
	return OAuthClient{}, contracts.ErrInvalid
}
