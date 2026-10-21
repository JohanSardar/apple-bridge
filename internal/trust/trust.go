package trust

import (
	"context"
	"crypto/ed25519"
	"crypto/hmac"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"crypto/x509/pkix"
	"encoding/base32"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"math/big"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Visibility map[contracts.Source]bool

func (v Visibility) Allows(r contracts.Record) bool {
	if !v[r.Source] {
		return false
	}
	if r.Collection != "system.command" || v["commands:all"] {
		return true
	}
	var command struct {
		Principal string `json:"principal"`
	}
	if json.Unmarshal(r.Data, &command) != nil {
		return false
	}
	if v[contracts.Source("principal:"+command.Principal)] {
		return true
	}
	parts := strings.Split(command.Principal, "/")
	return len(parts) == 2 && v[contracts.Source("principal:"+parts[1])]
}
func AllVisibility() Visibility {
	v := Visibility{"commands:all": true}
	for _, s := range contracts.Sources {
		v[s] = true
	}
	return v
}

type Principal struct {
	Remote      bool              `json:"remote,omitempty"`
	Name        string            `json:"name,omitempty"`
	Level       AgentLevel        `json:"level,omitempty"`
	CreatedAt   time.Time         `json:"created_at,omitempty"`
	RevokedAt   *time.Time        `json:"revoked_at,omitempty"`
	LastUsed    *time.Time        `json:"last_used,omitempty"`
	ExpiresAt   time.Time         `json:"expires_at,omitempty"`
	Resource    string            `json:"resource,omitempty"`
	Dial        string            `json:"dial,omitempty"`
	ID          string            `json:"id"`
	Kind        string            `json:"kind"`
	Grants      []contracts.Grant `json:"grants"`
	Fingerprint string            `json:"fingerprint,omitempty"`
	TokenHash   string            `json:"token_hash,omitempty"`
	Revision    uint64            `json:"revision"`
}

func (p Principal) Visibility() Visibility {
	v := Visibility{contracts.Source("principal:" + p.ID): true}
	if p.Kind == "replica" {
		v["commands:all"] = true
	}
	for _, g := range p.Grants {
		parts := strings.Split(string(g), ":")
		if g == "admin" {
			return AllVisibility()
		}
		if len(parts) == 2 && (parts[1] == "read" || parts[1] == "write") {
			v[contracts.Source(parts[0])] = true
		}
	}
	return v
}

func ParseGrants(scope string) ([]contracts.Grant, error) {
	if scope == "" {
		return nil, contracts.ErrInvalid
	}
	out := []contracts.Grant{}
	for _, s := range strings.Split(scope, ",") {
		if s == "admin" {
			out = append(out, "admin")
			continue
		}
		p := strings.Split(s, ":")
		if len(p) != 2 || (p[1] != "read" && p[1] != "write") {
			return nil, contracts.ErrInvalid
		}
		found := false
		for _, source := range contracts.Sources {
			if p[0] == string(source) {
				found = true
			}
		}
		if !found {
			return nil, contracts.ErrInvalid
		}
		out = append(out, contracts.Grant(s))
	}
	return out, nil
}

func Subset(grants, ceiling []contracts.Grant) bool {
	for _, g := range grants {
		allowed := false
		for _, c := range ceiling {
			if c == "admin" || c == g || strings.TrimSuffix(string(g), ":read")+":write" == string(c) {
				allowed = true
			}
		}
		if !allowed {
			return false
		}
	}
	return true
}

type PairCode struct {
	Remote      bool      `json:"remote,omitempty"`
	Dial        string    `json:"dial,omitempty"`
	ID          string    `json:"id"`
	Address     string    `json:"address"`
	Fingerprint string    `json:"fingerprint"`
	Secret      []byte    `json:"secret"`
	ExpiresAt   time.Time `json:"expires_at"`
}

type pairing struct {
	Dial      string            `json:"dial,omitempty"`
	Result    *Principal        `json:"result,omitempty"`
	Complete  bool              `json:"complete,omitempty"`
	Key       []byte            `json:"key"`
	ExpiresAt time.Time         `json:"expires_at"`
	Attempts  int               `json:"attempts"`
	Used      bool              `json:"used"`
	Grants    []contracts.Grant `json:"grants"`
}

type Enrollment struct {
	Remote      bool              `json:"remote,omitempty"`
	PairingID   string            `json:"pairing_id,omitempty"`
	Dial        string            `json:"dial,omitempty"`
	Ceiling     []contracts.Grant `json:"ceiling,omitempty"`
	ReplicaID   string            `json:"replica_id"`
	Address     string            `json:"address"`
	Fingerprint string            `json:"fingerprint"`
	Grants      []contracts.Grant `json:"grants"`
}

type credentials struct {
	OAuthAccess  []OAuthAccess  `json:"oauth_access,omitempty"`
	OAuthClients []OAuthClient  `json:"oauth_clients,omitempty"`
	OAuthCodes   []OAuthCode    `json:"oauth_codes,omitempty"`
	OAuthRefresh []OAuthRefresh `json:"oauth_refresh,omitempty"`

	Devices    []DeviceSession      `json:"devices,omitempty"`
	Passkeys   []Passkey            `json:"passkeys,omitempty"`
	WebEnroll  map[string]time.Time `json:"web_enroll,omitempty"`
	Principals []Principal          `json:"principals"`
	Pairings   map[string]pairing   `json:"pairings"`
	Enrollment *Enrollment          `json:"enrollment,omitempty"`
}

type Authority struct {
	home        string
	mu          sync.Mutex
	Certificate tls.Certificate
	Fingerprint string
}

func Open(home string) (*Authority, error) {
	if err := os.MkdirAll(home, 0700); err != nil {
		return nil, err
	}
	a := &Authority{home: home}
	err := a.update(func(d *credentials) error {
		certPath, keyPath := filepath.Join(home, "identity.pem"), filepath.Join(home, "identity-key.pem")
		if _, err := os.Stat(certPath); os.IsNotExist(err) {
			pub, key, err := ed25519.GenerateKey(rand.Reader)
			if err != nil {
				return err
			}
			serial, err := rand.Int(rand.Reader, new(big.Int).Lsh(big.NewInt(1), 128))
			if err != nil {
				return err
			}
			template := &x509.Certificate{SerialNumber: serial, Subject: pkix.Name{CommonName: "Apple Bridge"}, NotBefore: time.Now().Add(-time.Minute), NotAfter: time.Now().AddDate(5, 0, 0), KeyUsage: x509.KeyUsageDigitalSignature, ExtKeyUsage: []x509.ExtKeyUsage{x509.ExtKeyUsageServerAuth, x509.ExtKeyUsageClientAuth}, DNSNames: []string{"localhost"}}
			der, err := x509.CreateCertificate(rand.Reader, template, template, pub, key)
			if err != nil {
				return err
			}
			rawKey, err := x509.MarshalPKCS8PrivateKey(key)
			if err != nil {
				return err
			}
			if err = NewSecretStore(home).Put(context.Background(), "identity-key", pem.EncodeToMemory(&pem.Block{Type: "PRIVATE KEY", Bytes: rawKey})); err != nil {
				return err
			}
			if err = os.WriteFile(certPath, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der}), 0600); err != nil {
				return err
			}
		}
		var err error
		store := NewSecretStore(home)
		privateKey, err := store.Get(context.Background(), "identity-key")
		if errors.Is(err, contracts.ErrNotFound) {
			privateKey, err = os.ReadFile(keyPath)
			if err == nil {
				err = store.Put(context.Background(), "identity-key", privateKey)
			}
		}
		if err != nil {
			return err
		}
		certificate, err := os.ReadFile(certPath)
		if err != nil {
			return err
		}
		a.Certificate, err = tls.X509KeyPair(certificate, privateKey)
		if err == nil {
			os.Remove(keyPath)
		}
		if err != nil {
			return err
		}
		a.Fingerprint = Fingerprint(a.Certificate.Certificate[0])
		return nil
	})
	var storeError *CredentialStoreError
	if errors.As(err, &storeError) {
		report, _ := json.Marshal(storeError)
		os.WriteFile(filepath.Join(home, "credential-store-error.json"), report, 0600)
	} else if err == nil {
		os.Remove(filepath.Join(home, "credential-store-error.json"))
	}
	return a, err
}

func Fingerprint(b []byte) string { h := sha256.Sum256(b); return hex.EncodeToString(h[:]) }

func (a *Authority) update(fn func(*credentials) error) error {
	a.mu.Lock()
	defer a.mu.Unlock()
	lock, err := os.OpenFile(filepath.Join(a.home, "credentials.lock"), os.O_CREATE|os.O_RDWR, 0600)
	if err != nil {
		return err
	}
	defer lock.Close()
	if err = syscall.Flock(int(lock.Fd()), syscall.LOCK_EX); err != nil {
		return err
	}
	defer syscall.Flock(int(lock.Fd()), syscall.LOCK_UN)
	d := credentials{Pairings: map[string]pairing{}}
	path := filepath.Join(a.home, "credentials.json")
	b, err := os.ReadFile(path)
	if err == nil {
		if err = json.Unmarshal(b, &d); err != nil {
			return err
		}
	} else if !os.IsNotExist(err) {
		return err
	}
	result := fn(&d)
	b, err = json.Marshal(d)
	if err != nil {
		return err
	}
	temp, err := os.CreateTemp(a.home, "credentials-*")
	if err != nil {
		return err
	}
	defer os.Remove(temp.Name())
	if _, err = temp.Write(b); err != nil {
		temp.Close()
		return err
	}
	if err = temp.Sync(); err != nil {
		temp.Close()
		return err
	}
	if err = temp.Close(); err != nil {
		return err
	}
	if err = os.Rename(temp.Name(), path); err != nil {
		return err
	}
	dir, err := os.Open(a.home)
	if err != nil {
		return err
	}
	err = dir.Sync()
	dir.Close()
	if err != nil {
		return err
	}
	return result
}

func (a *Authority) Enrollment() (*Enrollment, error) { d, err := a.load(); return d.Enrollment, err }
func (a *Authority) SaveEnrollment(e Enrollment) error {
	return a.update(func(d *credentials) error {
		if d.Enrollment != nil && (!Subset(e.Grants, d.Enrollment.Grants) || !Subset(d.Enrollment.Grants, e.Grants)) {
			for i := range d.Principals {
				d.Principals[i].Revision++
			}
		}
		d.Enrollment = &e
		return nil
	})
}

func (a *Authority) CreateToken(grants []contracts.Grant) (string, error) {
	return a.CreateAPIToken(grants, false)
}
func (p Principal) RemoteAllowed() bool {
	return p.Kind == "agent" && principalActive(p) || p.Kind == "token" && p.Remote
}
func (a *Authority) CreateAPIToken(grants []contracts.Grant, remote bool) (string, error) {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		return "", err
	}
	token := "abt-" + hex.EncodeToString(b)
	err := a.update(func(d *credentials) error {
		if d.Enrollment != nil && !Subset(grants, d.Enrollment.Grants) {
			return contracts.ErrUnauthorized
		}
		d.Principals = append(d.Principals, Principal{ID: "token:" + contracts.NewID(""), Kind: "token", Remote: remote, Grants: grants, TokenHash: Fingerprint([]byte(token)), Revision: 1})
		return nil
	})
	return token, err
}

func (a *Authority) AuthenticateToken(token string) (Principal, error) {
	d, err := a.load()
	if err != nil {
		return Principal{}, err
	}
	hash := Fingerprint([]byte(token))
	p, err := credentialToken(d, hash)
	if err != nil || p.Kind != "agent" {
		return p, err
	}
	err = a.update(func(d *credentials) error {
		current, err := credentialToken(*d, hash)
		if err != nil {
			return err
		}
		p = current
		now := time.Now().UTC()
		for i, candidate := range d.Principals {
			if candidate.ID == p.ID {
				d.Principals[i].LastUsed = &now
				p.LastUsed = &now
				return nil
			}
		}
		return contracts.ErrUnauthorized
	})
	return p, err
}

func credentialToken(d credentials, hash string) (Principal, error) {
	for _, p := range d.Principals {
		if p.Kind != "token" && p.Kind != "agent" || !principalActive(p) {
			continue
		}
		matched := hmac.Equal([]byte(hash), []byte(p.TokenHash))
		if !matched && p.Kind == "agent" {
			for _, access := range d.OAuthAccess {
				if access.Principal == p.ID && time.Now().Before(access.ExpiresAt) && hmac.Equal([]byte(hash), []byte(access.Hash)) {
					matched = true
					p.ExpiresAt = access.ExpiresAt
					p.Resource = access.Resource
					break
				}
			}
		}
		if !matched {
			continue
		}
		if d.Enrollment != nil {
			p.Grants = EffectiveGrants(p.Grants, d.Enrollment.Grants)
		}
		return p, nil
	}
	return Principal{}, contracts.ErrUnauthorized
}

func (a *Authority) AuthenticateCertificate(cert *x509.Certificate) (Principal, error) {
	var p Principal
	if time.Now().Before(cert.NotBefore) || time.Now().After(cert.NotAfter) {
		return p, contracts.ErrExpired
	}
	d, err := a.load()
	if err != nil {
		return p, err
	}
	err = func() error {
		hash := Fingerprint(cert.Raw)
		for _, candidate := range d.Principals {
			if candidate.Kind == "replica" && candidate.Fingerprint == hash {
				p = candidate
				return nil
			}
		}
		return contracts.ErrUnauthorized
	}()
	return p, err
}

func (a *Authority) Pair(address string, grants []contracts.Grant) (string, error) {
	return a.PairTTL(address, grants, 10*time.Minute)
}
func (a *Authority) PairTTL(address string, grants []contracts.Grant, ttl time.Duration) (string, error) {
	return a.PairDial(address, "", grants, ttl)
}
func (a *Authority) PairDial(address, dial string, grants []contracts.Grant, ttl time.Duration) (string, error) {
	if dial != "" {
		host, port, err := net.SplitHostPort(dial)
		if err != nil || host == "" || port == "" {
			return "", contracts.ErrInvalid
		}
	}
	if ttl <= 0 || ttl > 10*time.Minute {
		return "", contracts.ErrInvalid
	}
	secret := make([]byte, 20)
	if _, err := rand.Read(secret); err != nil {
		return "", err
	}
	code := PairCode{Dial: dial, ID: contracts.NewID(""), Address: address, Fingerprint: a.Fingerprint, Secret: secret, ExpiresAt: time.Now().Add(ttl)}
	key := sha256.Sum256(secret)
	err := a.update(func(d *credentials) error {
		d.Pairings[code.ID] = pairing{Dial: dial, Key: key[:], ExpiresAt: code.ExpiresAt, Grants: grants}
		return nil
	})
	if err != nil {
		return "", err
	}
	return encodePairCode(code)
}

type JoinRequest struct {
	ID    string `json:"id"`
	Proof []byte `json:"proof"`
}

func PairProof(secret, cert []byte) []byte {
	key := sha256.Sum256(secret)
	m := hmac.New(sha256.New, key[:])
	m.Write([]byte("apple-bridge-pair/1\x00"))
	m.Write(cert)
	return m.Sum(nil)
}

func (a *Authority) PairHandler() http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != "POST" || r.TLS == nil || len(r.TLS.PeerCertificates) != 1 {
			http.Error(w, "unauthorized", 401)
			return
		}
		var req JoinRequest
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096)).Decode(&req) != nil {
			http.Error(w, "invalid", 400)
			return
		}
		cert := r.TLS.PeerCertificates[0]
		p, err := a.acceptPair(req, cert, false)
		if err != nil {
			http.Error(w, "unauthorized", 401)
			return
		}
		w.Header().Set("Content-Type", "application/json")
		json.NewEncoder(w).Encode(p)
	})
}

func DecodePairCode(code string) (PairCode, error) {
	var c PairCode
	if !strings.HasPrefix(code, "ab1-") {
		return c, contracts.ErrInvalid
	}
	b, err := base32.StdEncoding.WithPadding(base32.NoPadding).DecodeString(strings.TrimPrefix(code, "ab1-"))
	if err != nil {
		return c, contracts.ErrInvalid
	}
	if json.Unmarshal(b, &c) != nil || c.ID == "" || c.Address == "" || len(c.Fingerprint) != 64 || len(c.Secret) != 20 {
		return c, contracts.ErrInvalid
	}
	if c.Remote {
		if _, err := relayURL(c.Address); err != nil || c.Dial != "" {
			return c, contracts.ErrInvalid
		}
	}
	if time.Now().After(c.ExpiresAt) {
		return c, contracts.ErrExpired
	}
	return c, nil
}

func (a *Authority) ClientTLS(fingerprint string) *tls.Config {
	return &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{a.Certificate}, InsecureSkipVerify: true, VerifyConnection: func(cs tls.ConnectionState) error {
		if len(cs.PeerCertificates) == 0 || Fingerprint(cs.PeerCertificates[0].Raw) != fingerprint {
			return contracts.ErrUnauthorized
		}
		cert := cs.PeerCertificates[0]
		if time.Now().Before(cert.NotBefore) || time.Now().After(cert.NotAfter) {
			return contracts.ErrExpired
		}
		return nil
	}}
}

func (a *Authority) Join(ctx context.Context, code string) error {
	c, err := DecodePairCode(code)
	if err != nil {
		return err
	}
	if c.Remote {
		return a.joinRemote(ctx, c)
	}
	if c.Dial != "" {
		return contracts.ErrInvalid
	}
	body, err := json.Marshal(JoinRequest{ID: c.ID, Proof: PairProof(c.Secret, a.Certificate.Certificate[0])})
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, "POST", "https://"+c.Address+"/v1/pair", strings.NewReader(string(body)))
	if err != nil {
		return err
	}
	transport := &http.Transport{TLSClientConfig: a.ClientTLS(c.Fingerprint)}
	defer transport.CloseIdleConnections()
	client := &http.Client{Transport: transport, Timeout: 15 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return contracts.ErrUnauthorized
	}
	var p Principal
	if err = json.NewDecoder(resp.Body).Decode(&p); err != nil {
		return err
	}
	return a.SaveEnrollment(Enrollment{PairingID: c.ID, ReplicaID: p.ID, Address: c.Address, Fingerprint: c.Fingerprint, Grants: p.Grants, Ceiling: p.Grants})
}

func (a *Authority) acceptPair(req JoinRequest, cert *x509.Certificate, resume bool) (Principal, error) {
	var p Principal
	err := a.update(func(d *credentials) error {
		pair, ok := d.Pairings[req.ID]
		if !ok || (pair.Used && !resume) || (!pair.Used && pair.Attempts >= 5) || time.Now().After(pair.ExpiresAt) {
			return contracts.ErrUnauthorized
		}
		if !pair.Used {
			pair.Attempts++
		}
		d.Pairings[req.ID] = pair
		m := hmac.New(sha256.New, pair.Key)
		m.Write([]byte("apple-bridge-pair/1\x00"))
		m.Write(cert.Raw)
		if !hmac.Equal(m.Sum(nil), req.Proof) {
			return contracts.ErrUnauthorized
		}
		if time.Now().After(cert.NotAfter) || time.Now().Before(cert.NotBefore) {
			return contracts.ErrExpired
		}
		if pair.Used {
			if pair.Result == nil || pair.Result.Fingerprint != Fingerprint(cert.Raw) {
				return contracts.ErrUnauthorized
			}
			for _, current := range d.Principals {
				if current.ID == pair.Result.ID {
					p = current
					return nil
				}
			}
			return contracts.ErrUnauthorized
		}
		p = Principal{Dial: pair.Dial, ID: "replica:" + contracts.NewID(""), Kind: "replica", Grants: pair.Grants, Fingerprint: Fingerprint(cert.Raw), Revision: 1}
		d.Principals = append(d.Principals, p)
		pair.Used = true
		if pair.Dial == "" {
			pair.Key = nil
		} else {
			pair.Result = &p
		}
		d.Pairings[req.ID] = pair
		return nil
	})
	return p, err
}
