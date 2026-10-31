package trust

import (
	"crypto/x509"
	"strings"
	"testing"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/go-webauthn/webauthn/webauthn"
)

func securityAuthority(t *testing.T) *Authority {
	t.Helper()
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	a, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	return a
}

func TestPasskeyCeremonyCannotOutliveOwnerSession(t *testing.T) {
	for _, mode := range []string{"revoked", "expired"} {
		t.Run(mode, func(t *testing.T) {
			a := securityAuthority(t)
			token, session, err := a.CreateDeviceSession("public")
			if err != nil {
				t.Fatal(err)
			}
			p, _, err := a.AuthenticateDevice(token)
			if err != nil {
				t.Fatal(err)
			}
			if mode == "revoked" {
				err = a.Revoke(session.DeviceID)
			} else {
				err = a.update(func(d *credentials) error { d.Devices[0].ExpiresAt = time.Now().Add(-time.Second); return nil })
			}
			if err != nil {
				t.Fatal(err)
			}
			if a.SavePasskey(Passkey{ID: "[ab-test]", RPID: "example.com", Credential: webauthn.Credential{}}, "", &p) == nil {
				t.Fatal("inactive owner session enrolled a persistent passkey")
			}
		})
	}
}

func TestReplicaAuthenticationRejectsInactivePrincipal(t *testing.T) {
	for _, mode := range []string{"revoked", "expired"} {
		t.Run(mode, func(t *testing.T) {
			a := securityAuthority(t)
			cert, err := x509.ParseCertificate(a.Certificate.Certificate[0])
			if err != nil {
				t.Fatal(err)
			}
			p := Principal{ID: "replica:[ab-test]", Kind: "replica", Fingerprint: a.Fingerprint, Grants: []contracts.Grant{"messages:read"}}
			if mode == "revoked" {
				now := time.Now()
				p.RevokedAt = &now
			} else {
				p.ExpiresAt = time.Now().Add(-time.Second)
			}
			if err = a.update(func(d *credentials) error { d.Principals = append(d.Principals, p); return nil }); err != nil {
				t.Fatal(err)
			}
			if _, err = a.AuthenticateCertificate(cert); err == nil {
				t.Fatal("inactive replica authenticated")
			}
		})
	}
}

func TestDelegatedScopeCannotExpandToReplicaScope(t *testing.T) {
	a := securityAuthority(t)
	replica := Principal{ID: "replica:[ab-test]", Kind: "replica", Fingerprint: a.Fingerprint, Grants: []contracts.Grant{"messages:read", "contacts:read"}}
	if err := a.update(func(d *credentials) error { d.Principals = append(d.Principals, replica); return nil }); err != nil {
		t.Fatal(err)
	}
	delegated := Principal{ID: replica.ID + "/agent:test", Kind: "delegated", Fingerprint: a.Fingerprint, Grants: []contracts.Grant{"messages:read"}, Level: AgentRead}
	if a.Authorize(delegated, "messages:read") != nil {
		t.Fatal("valid delegated scope rejected")
	}
	if a.Authorize(delegated, "contacts:read") == nil {
		t.Fatal("delegated caller borrowed replica scope")
	}
}

func TestOAuthExchangeRejectsExpiredPrincipal(t *testing.T) {
	a := securityAuthority(t)
	agent, _, err := a.CreateAgent("[ab-test]", AgentRead, []contracts.Grant{"messages:read"})
	if err != nil {
		t.Fatal(err)
	}
	if err = a.update(func(d *credentials) error {
		for i := range d.Principals {
			if d.Principals[i].ID == agent.ID {
				d.Principals[i].ExpiresAt = time.Now().Add(-time.Second)
			}
		}
		d.OAuthRefresh = append(d.OAuthRefresh, OAuthRefresh{Hash: Fingerprint([]byte("[ab-test] refresh")), ClientID: "client", Resource: "https://example.com/mcp", Principal: agent.ID, ExpiresAt: time.Now().Add(time.Hour)})
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	tokens, err := a.ExchangeOAuth("refresh_token", "client", "", "", "", "https://example.com/mcp", "[ab-test] refresh", "")
	if err == nil || strings.TrimSpace(tokens.AccessToken) != "" {
		t.Fatal("expired agent minted tokens")
	}
}

func TestActiveOwnerCanEnrollAndEnrollmentTokensAreSingleUse(t *testing.T) {
	a := securityAuthority(t)
	token, _, err := a.CreateDeviceSession("trusted")
	if err != nil {
		t.Fatal(err)
	}
	p, _, err := a.AuthenticateDevice(token)
	if err != nil {
		t.Fatal(err)
	}
	if err = a.SavePasskey(Passkey{ID: "[ab-test] owner", RPID: "example.com"}, "", &p); err != nil {
		t.Fatal("active owner could not enroll")
	}
	enroll, err := a.CreateWebEnroll()
	if err != nil {
		t.Fatal(err)
	}
	if err = a.SavePasskey(Passkey{ID: "[ab-test] initial", RPID: "example.com"}, enroll, nil); err != nil {
		t.Fatal("enrollment failed")
	}
	if a.SavePasskey(Passkey{ID: "[ab-test] replay", RPID: "example.com"}, enroll, nil) == nil {
		t.Fatal("enrollment replay accepted")
	}
	expired, err := a.CreateWebEnroll()
	if err != nil {
		t.Fatal(err)
	}
	if err = a.update(func(d *credentials) error {
		d.WebEnroll[Fingerprint([]byte(expired))] = time.Now().Add(-time.Second)
		return nil
	}); err != nil {
		t.Fatal(err)
	}
	if a.SavePasskey(Passkey{ID: "[ab-test] expired", RPID: "example.com"}, expired, nil) == nil {
		t.Fatal("expired enrollment accepted")
	}
}
