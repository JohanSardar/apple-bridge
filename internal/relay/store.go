package relay

import (
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

var ErrDenied = errors.New("relay access denied")

type Limits struct {
	Connections    int   `json:"connections"`
	BytesPerSecond int64 `json:"bytes_per_second"`
	MonthlyBytes   int64 `json:"monthly_bytes"`
}
type Device struct {
	Certificates   []string `json:"certificates,omitempty"`
	Domain         string   `json:"domain,omitempty"`
	PublicKey      string   `json:"public_key"`
	Hostname       string   `json:"hostname"`
	CredentialHash string   `json:"credential_hash"`
	Revoked        bool     `json:"revoked"`
	Disabled       bool     `json:"disabled"`
	Limits         Limits   `json:"limits"`
	Month          string   `json:"month"`
	Bytes          int64    `json:"bytes"`
}
type Enrollment struct {
	PublicKey string `json:"public_key"`
	Nonce     string `json:"nonce"`
	Signature string `json:"signature"`
	Hostname  string `json:"hostname,omitempty"`
}
type Credential struct {
	Hostname   string `json:"hostname"`
	Credential string `json:"credential"`
}
type Store struct {
	mu         sync.Mutex
	path       string
	zone       string
	defaults   Limits
	devices    map[string]Device
	challenges map[string]time.Time
}

func Open(path, zone string, defaults Limits) (*Store, error) {
	zone = strings.ToLower(strings.TrimSuffix(zone, "."))
	if path == "" || len(zone) > 228 || !validHost(zone) || defaults.Connections < 1 || defaults.BytesPerSecond < 1 || defaults.MonthlyBytes < 1 {
		return nil, errors.New("invalid relay configuration")
	}
	s := &Store{path: path, zone: zone, defaults: defaults, devices: map[string]Device{}, challenges: map[string]time.Time{}}
	data, err := os.ReadFile(path)
	if err == nil {
		err = json.Unmarshal(data, &s.devices)
	}
	if err != nil && !os.IsNotExist(err) {
		return nil, err
	}
	if s.devices == nil {
		return nil, errors.New("invalid relay state")
	}
	for host, d := range s.devices {
		if !validHost(host) || d.Hostname != host || d.Bytes < 0 || d.Limits.Connections < 1 || d.Limits.BytesPerSecond < 1 || d.Limits.MonthlyBytes < 1 || !strings.HasSuffix(host, "."+zone) {
			return nil, errors.New("state zone mismatch")
		}
	}
	return s, nil
}
func randomToken() string {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		panic(err)
	}
	return base64.RawURLEncoding.EncodeToString(b)
}
func digest(v string) string { h := sha256.Sum256([]byte(v)); return hex.EncodeToString(h[:]) }
func (s *Store) save() error {
	if err := os.MkdirAll(filepath.Dir(s.path), 0700); err != nil {
		return err
	}
	data, err := json.Marshal(s.devices)
	if err != nil {
		return err
	}
	f, err := os.CreateTemp(filepath.Dir(s.path), ".relay-*")
	if err != nil {
		return err
	}
	defer os.Remove(f.Name())
	if _, err = f.Write(data); err == nil {
		err = f.Sync()
	}
	closeErr := f.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	if err = os.Rename(f.Name(), s.path); err != nil {
		return err
	}
	dir, err := os.Open(filepath.Dir(s.path))
	if err != nil {
		return err
	}
	defer dir.Close()
	return dir.Sync()
}
func (s *Store) Challenge() (string, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	now := time.Now()
	for n, t := range s.challenges {
		if now.After(t) {
			delete(s.challenges, n)
		}
	}
	if len(s.challenges) >= 4096 {
		return "", ErrDenied
	}
	n := randomToken()
	s.challenges[n] = now.Add(time.Minute)
	return n, nil
}
func EnrollmentMessage(zone, nonce string) []byte {
	return []byte("apple-bridge-relay-enroll\n" + zone + "\n" + nonce)
}
func (s *Store) Enroll(in Enrollment) (Credential, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	expires, ok := s.challenges[in.Nonce]
	delete(s.challenges, in.Nonce)
	key, e1 := base64.RawURLEncoding.DecodeString(in.PublicKey)
	sig, e2 := base64.RawURLEncoding.DecodeString(in.Signature)
	if !ok || time.Now().After(expires) || e1 != nil || e2 != nil || len(key) != ed25519.PublicKeySize || base64.RawURLEncoding.EncodeToString(key) != in.PublicKey || !ed25519.Verify(key, EnrollmentMessage(s.zone, in.Nonce), sig) {
		return Credential{}, ErrDenied
	}
	host := ""
	for h, d := range s.devices {
		if d.PublicKey == in.PublicKey {
			host = h
			break
		}
	}
	if in.Hostname != "" && in.Hostname != host {
		return Credential{}, ErrDenied
	}
	if host == "" {
		for {
			host = digest(randomToken())[:24] + "." + s.zone
			if _, exists := s.devices[host]; !exists {
				break
			}
		}
	}
	d, exists := s.devices[host]
	if d.Revoked {
		return Credential{}, ErrDenied
	}
	if !exists {
		d = Device{PublicKey: in.PublicKey, Hostname: host, Limits: s.defaults, Month: time.Now().UTC().Format("2006-01")}
	}
	token := randomToken()
	old := s.devices[host]
	d.CredentialHash = digest(token)
	s.devices[host] = d
	if err := s.save(); err != nil {
		if exists {
			s.devices[host] = old
		} else {
			delete(s.devices, host)
		}
		return Credential{}, err
	}
	return Credential{Hostname: host, Credential: token}, nil
}
func (s *Store) authenticated(host, token string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	d, ok := s.devices[host]
	return ok && !d.Revoked && !d.Disabled && d.CredentialHash == digest(token)
}
func (s *Store) Policy(host string, limits Limits, disabled, revoked bool) error {
	s.mu.Lock()
	defer s.mu.Unlock()
	d, ok := s.devices[host]
	if !ok || limits.Connections < 1 || limits.BytesPerSecond < 1 || limits.MonthlyBytes < 1 {
		return ErrDenied
	}
	old := d
	d.Limits = limits
	d.Disabled = disabled
	d.Revoked = revoked || d.Revoked
	s.devices[host] = d
	if err := s.save(); err != nil {
		s.devices[host] = old
		return err
	}
	return nil
}
func (s *Store) Device(host string) (Device, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()
	d, ok := s.devices[host]
	return d, ok
}
func (s *Store) charge(host string, n int) (int, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	d, ok := s.devices[host]
	if !ok || d.Disabled || d.Revoked {
		return 0, ErrDenied
	}
	month := time.Now().UTC().Format("2006-01")
	if d.Month != month {
		d.Month = month
		d.Bytes = 0
	}
	remaining := d.Limits.MonthlyBytes - d.Bytes
	if remaining <= 0 {
		return 0, ErrDenied
	}
	if int64(n) > remaining {
		n = int(remaining)
	}
	old := s.devices[host]
	d.Bytes += int64(n)
	s.devices[host] = d
	if err := s.save(); err != nil {
		s.devices[host] = old
		return 0, err
	}
	return n, nil
}
