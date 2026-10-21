package trust

import (
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"slices"
	"strings"
	"time"
)

func (a *Authority) IssueOAuthCode(c OAuthCode) (string, error) {
	raw, err := randomWebToken(32)
	if err != nil {
		return "", err
	}
	c.Hash = Fingerprint([]byte(raw))
	c.ExpiresAt = time.Now().Add(5 * time.Minute)
	err = a.update(func(d *credentials) error {
		d.OAuthCodes = slices.DeleteFunc(d.OAuthCodes, func(v OAuthCode) bool { return time.Now().After(v.ExpiresAt) })
		if len(d.OAuthCodes) >= 1000 {
			return contracts.ErrUnavailable
		}
		d.OAuthCodes = append(d.OAuthCodes, c)
		return nil
	})
	return raw, err
}
func validVerifier(v string) bool {
	if len(v) < 43 || len(v) > 128 {
		return false
	}
	for _, c := range v {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || strings.ContainsRune("-._~", c)) {
			return false
		}
	}
	return true
}
func (a *Authority) ExchangeOAuth(grant, client, code, redirect, verifier, resource, refresh, scope string) (OAuthTokens, error) {
	var out OAuthTokens
	access, err := randomWebToken(32)
	if err != nil {
		return out, err
	}
	next, err := randomWebToken(32)
	if err != nil {
		return out, err
	}
	err = a.update(func(d *credentials) error {
		principal := ""
		expires := time.Now().Add(90 * 24 * time.Hour)
		if grant == "authorization_code" {
			hash := Fingerprint([]byte(code))
			challenge := sha256.Sum256([]byte(verifier))
			for i, c := range d.OAuthCodes {
				if c.Hash != hash {
					continue
				}
				if c.ClientID != client || c.Resource != resource || c.RedirectURI != redirect || !validVerifier(verifier) || subtle.ConstantTimeCompare([]byte(c.Challenge), []byte(base64.RawURLEncoding.EncodeToString(challenge[:]))) != 1 || time.Now().After(c.ExpiresAt) {
					return contracts.ErrUnauthorized
				}
				if c.Used {
					revokeOAuth(d, c.Principal)
					return contracts.ErrUnauthorized
				}
				d.OAuthCodes[i].Used = true
				principal = c.Principal
				break
			}
		} else if grant == "refresh_token" {
			hash := Fingerprint([]byte(refresh))
			for i, f := range d.OAuthRefresh {
				if f.Hash != hash {
					continue
				}
				if f.ClientID != client || f.Resource != resource || time.Now().After(f.ExpiresAt) {
					return contracts.ErrUnauthorized
				}
				if f.Used {
					revokeOAuth(d, f.Principal)
					return contracts.ErrUnauthorized
				}
				d.OAuthRefresh[i].Used = true
				principal = f.Principal
				expires = f.ExpiresAt
				break
			}
		} else {
			return contracts.ErrInvalid
		}
		if principal == "" {
			return contracts.ErrUnauthorized
		}
		for i, p := range d.Principals {
			if p.ID != principal || p.Kind != "agent" || p.RevokedAt != nil {
				continue
			}
			if scope != "" && scope != strings.Join(grantStrings(p.Grants), " ") {
				return contracts.ErrInvalid
			}
			p.TokenHash = ""
			d.OAuthAccess = slices.DeleteFunc(d.OAuthAccess, func(v OAuthAccess) bool { return time.Now().After(v.ExpiresAt) })
			d.OAuthAccess = append(d.OAuthAccess, OAuthAccess{Hash: Fingerprint([]byte(access)), Principal: principal, Resource: resource, ExpiresAt: time.Now().Add(time.Hour)})
			p.Resource = resource
			d.Principals[i] = p
			d.OAuthRefresh = slices.DeleteFunc(d.OAuthRefresh, func(v OAuthRefresh) bool { return time.Now().After(v.ExpiresAt) })
			d.OAuthRefresh = append(d.OAuthRefresh, OAuthRefresh{Hash: Fingerprint([]byte(next)), ClientID: client, Resource: resource, Principal: principal, ExpiresAt: expires})
			out = OAuthTokens{access, "Bearer", 3600, next, strings.Join(grantStrings(p.Grants), " ")}
			return nil
		}
		return contracts.ErrUnauthorized
	})
	return out, err
}
func grantStrings(grants []contracts.Grant) []string {
	out := []string{}
	for _, g := range grants {
		out = append(out, string(g))
	}
	return out
}
func revokeOAuth(d *credentials, id string) {
	for i, p := range d.Principals {
		if p.ID == id {
			now := time.Now().UTC()
			d.Principals[i].RevokedAt = &now
			d.Principals[i].TokenHash = ""
			d.Principals[i].Revision++
		}
	}
}
