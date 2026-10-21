package trust

import (
	"crypto/rand"
	"encoding/base64"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/go-webauthn/webauthn/webauthn"
	"time"
)

type DeviceSession struct {
	DeviceID  string    `json:"device_id"`
	Kind      string    `json:"kind"`
	ExpiresAt time.Time `json:"expires_at"`
	Cache     string    `json:"cache"`
	CreatedAt time.Time `json:"created_at"`
	LastSeen  time.Time `json:"last_seen"`
	TokenHash string    `json:"token_hash,omitempty"`
}

type Passkey struct {
	ID         string              `json:"id"`
	RPID       string              `json:"rp_id"`
	Credential webauthn.Credential `json:"credential"`
	CreatedAt  time.Time           `json:"created_at"`
}

func randomWebToken(size int) (string, error) {
	b := make([]byte, size)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	return base64.RawURLEncoding.EncodeToString(b), nil
}

func (a *Authority) CreateWebEnroll() (string, error) {
	token, err := randomWebToken(20)
	if err != nil {
		return "", err
	}
	err = a.update(func(d *credentials) error {
		if d.WebEnroll == nil {
			d.WebEnroll = map[string]time.Time{}
		}
		for hash, expiry := range d.WebEnroll {
			if time.Now().After(expiry) {
				delete(d.WebEnroll, hash)
			}
		}
		d.WebEnroll[Fingerprint([]byte(token))] = time.Now().Add(10 * time.Minute)
		return nil
	})
	return token, err
}

func (a *Authority) CheckWebEnroll(token string) error {
	d, err := a.load()
	if err != nil {
		return err
	}
	expiry, ok := d.WebEnroll[Fingerprint([]byte(token))]
	if !ok || time.Now().After(expiry) {
		return contracts.ErrUnauthorized
	}
	return nil
}

func (a *Authority) SavePasskey(key Passkey, enroll string, principal *Principal) error {
	return a.update(func(d *credentials) error {
		if principal == nil {
			hash := Fingerprint([]byte(enroll))
			expiry, ok := d.WebEnroll[hash]
			if !ok || time.Now().After(expiry) {
				return contracts.ErrUnauthorized
			}
			delete(d.WebEnroll, hash)
		} else {
			found := false
			for _, p := range d.Principals {
				if p.ID == principal.ID && p.Kind == "device" {
					found = true
				}
			}
			if !found {
				return contracts.ErrUnauthorized
			}
		}
		for _, existing := range d.Passkeys {
			if existing.ID == key.ID {
				return contracts.ErrConflict
			}
		}
		d.Passkeys = append(d.Passkeys, key)
		return nil
	})
}

func (a *Authority) Passkeys(rp string) ([]Passkey, error) {
	d, err := a.load()
	keys := []Passkey{}
	for _, key := range d.Passkeys {
		if key.RPID == rp {
			keys = append(keys, key)
		}
	}
	return keys, err
}

func (a *Authority) UpdatePasskey(rp string, key webauthn.Credential) error {
	return a.update(func(d *credentials) error {
		id := base64.RawURLEncoding.EncodeToString(key.ID)
		for i, k := range d.Passkeys {
			if k.RPID == rp && k.ID == id {
				d.Passkeys[i].Credential = key
				return nil
			}
		}
		return contracts.ErrUnauthorized
	})
}

func (a *Authority) DeletePasskey(rp, id string) error {
	return a.update(func(d *credentials) error {
		for i, k := range d.Passkeys {
			if k.RPID == rp && k.ID == id {
				d.Passkeys = append(d.Passkeys[:i], d.Passkeys[i+1:]...)
				return nil
			}
		}
		return contracts.ErrNotFound
	})
}

func (a *Authority) CreateDeviceSession(kind string) (string, DeviceSession, error) {
	if kind != "trusted" && kind != "public" {
		return "", DeviceSession{}, contracts.ErrInvalid
	}
	token, err := randomWebToken(32)
	if err != nil {
		return "", DeviceSession{}, err
	}
	now := time.Now().UTC()
	session := DeviceSession{DeviceID: "device:" + contracts.NewID(""), Kind: kind, Cache: "persistent", CreatedAt: now, LastSeen: now, ExpiresAt: now.Add(30 * 24 * time.Hour), TokenHash: Fingerprint([]byte(token))}
	if kind == "public" {
		session.Cache = "memory"
		session.ExpiresAt = now.Add(30 * time.Minute)
	}
	err = a.update(func(d *credentials) error {
		grants := []contracts.Grant{"admin"}
		if d.Enrollment != nil {
			grants = d.Enrollment.Grants
		}
		d.Principals = append(d.Principals, Principal{ID: session.DeviceID, Kind: "device", Grants: grants, Revision: 1})
		d.Devices = append(d.Devices, session)
		return nil
	})
	session.TokenHash = ""
	return token, session, err
}

func validDevice(s DeviceSession, now time.Time) bool {
	absolute := 90 * 24 * time.Hour
	if s.Kind == "public" {
		absolute = 12 * time.Hour
	}
	return now.Before(s.ExpiresAt) && now.Before(s.CreatedAt.Add(absolute))
}

func (a *Authority) AuthenticateDevice(token string) (Principal, DeviceSession, error) {
	return a.deviceSession(token, true)
}
func (a *Authority) InspectDevice(token string) (Principal, DeviceSession, error) {
	return a.deviceSession(token, false)
}
func (a *Authority) deviceSession(token string, touch bool) (Principal, DeviceSession, error) {
	d, err := a.load()
	if err != nil {
		return Principal{}, DeviceSession{}, err
	}
	now := time.Now().UTC()
	hash := Fingerprint([]byte(token))
	for _, session := range d.Devices {
		if session.TokenHash != hash || !validDevice(session, now) {
			continue
		}
		p, err := a.Principal(session.DeviceID)
		if err != nil {
			return p, session, err
		}
		if d.Enrollment != nil {
			p.Grants = EffectiveGrants(p.Grants, d.Enrollment.Grants)
		}
		if touch && now.Sub(session.LastSeen) >= time.Minute {
			err = a.update(func(current *credentials) error {
				for i, s := range current.Devices {
					if s.DeviceID == session.DeviceID && validDevice(s, now) {
						s.LastSeen = now
						idle, absolute := 30*24*time.Hour, 90*24*time.Hour
						if s.Kind == "public" {
							idle, absolute = 30*time.Minute, 12*time.Hour
						}
						s.ExpiresAt = now.Add(idle)
						if s.ExpiresAt.After(s.CreatedAt.Add(absolute)) {
							s.ExpiresAt = s.CreatedAt.Add(absolute)
						}
						current.Devices[i] = s
						session = s
						return nil
					}
				}
				return contracts.ErrUnauthorized
			})
		}
		session.TokenHash = ""
		return p, session, err
	}
	return Principal{}, DeviceSession{}, contracts.ErrUnauthorized
}

func (a *Authority) DeviceSessions() ([]DeviceSession, error) {
	d, err := a.load()
	sessions := []DeviceSession{}
	live := map[string]bool{}
	for _, p := range d.Principals {
		live[p.ID] = true
	}
	for _, s := range d.Devices {
		if validDevice(s, time.Now()) && live[s.DeviceID] {
			s.TokenHash = ""
			sessions = append(sessions, s)
		}
	}
	return sessions, err
}

func (a *Authority) LockWeb() error {
	return a.update(func(d *credentials) error {
		kept := d.Principals[:0]
		for _, p := range d.Principals {
			if p.Kind != "device" {
				kept = append(kept, p)
			}
		}
		d.Principals = kept
		d.Devices = nil
		return nil
	})
}
