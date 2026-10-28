package remote

import (
	"bytes"
	"context"
	"crypto/ecdsa"
	"crypto/ed25519"
	"crypto/elliptic"
	"crypto/rand"
	"crypto/sha256"
	"crypto/tls"
	"crypto/x509"
	"encoding/hex"
	"encoding/json"
	"encoding/pem"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/relay"
	"golang.org/x/crypto/acme"
)

type CertificateOptions struct {
	DirectoryURL string
	HTTPClient   *http.Client
}

func ProvisionCertificate(ctx context.Context, home, host, endpoint string, key ed25519.PrivateKey, credential relay.Credential, options CertificateOptions) error {
	manager := &certificateManager{home: home, host: host, identity: relayIdentity{Key: key, Endpoint: endpoint, Credential: credential}, options: options}
	return manager.ensure(ctx)
}
func (m *certificateManager) client() *http.Client {
	if m.options.HTTPClient != nil {
		return m.options.HTTPClient
	}
	return http.DefaultClient
}

type certificateManager struct {
	options     CertificateOptions
	mu          sync.RWMutex
	certificate *tls.Certificate
	home        string
	identity    relayIdentity
	host        string
}

func (m *certificateManager) get(*tls.ClientHelloInfo) (*tls.Certificate, error) {
	m.mu.RLock()
	defer m.mu.RUnlock()
	if m.certificate == nil {
		return nil, fmt.Errorf("certificate unavailable")
	}
	return m.certificate, nil
}
func (m *certificateManager) ensure(ctx context.Context) error {
	path := filepath.Join(m.home, "remote")
	certPath, keyPath := filepath.Join(path, "relay-cert.pem"), filepath.Join(path, "relay-key.pem")
	if c, err := tls.LoadX509KeyPair(certPath, keyPath); err == nil {
		leaf, e := x509.ParseCertificate(c.Certificate[0])
		if e == nil && time.Now().After(leaf.NotBefore) && leaf.VerifyHostname(m.host) == nil && time.Until(leaf.NotAfter) > 30*24*time.Hour {
			if err = m.reportCertificate(ctx, certificateFingerprint(leaf.Raw)); err != nil {
				return err
			}
			m.mu.Lock()
			m.certificate = &c
			m.mu.Unlock()
			return nil
		}
	}
	key, err := loadCertificateKey(filepath.Join(path, "acme-account.pem"))
	if err != nil {
		return err
	}
	directory := m.options.DirectoryURL
	if directory == "" {
		directory = os.Getenv("APPLE_BRIDGE_ACME_DIRECTORY")
	}
	if directory == "" {
		directory = acme.LetsEncryptURL
	}
	client := &acme.Client{Key: key, DirectoryURL: directory, HTTPClient: m.client()}
	if _, err = client.Register(ctx, &acme.Account{}, acme.AcceptTOS); err != nil && !errors.Is(err, acme.ErrAccountAlreadyExists) {
		return err
	}
	order, err := client.AuthorizeOrder(ctx, acme.DomainIDs(m.host))
	if err != nil {
		return err
	}
	for _, url := range order.AuthzURLs {
		auth, err := client.GetAuthorization(ctx, url)
		if err != nil {
			return err
		}
		if auth.Status == acme.StatusValid {
			continue
		}
		var challenge *acme.Challenge
		for _, c := range auth.Challenges {
			if c.Type == "dns-01" {
				challenge = c
				break
			}
		}
		if challenge == nil {
			return fmt.Errorf("DNS-01 challenge unavailable")
		}
		value, err := client.DNS01ChallengeRecord(challenge.Token)
		if err != nil {
			return err
		}
		if err = m.publishTXT(ctx, value); err != nil {
			return err
		}
		defer func() {
			cleanup, cancel := context.WithTimeout(context.Background(), 15*time.Second)
			defer cancel()
			m.publishTXT(cleanup, "")
		}()
		if _, err = client.Accept(ctx, challenge); err != nil {
			return err
		}
		if _, err = client.WaitAuthorization(ctx, url); err != nil {
			return err
		}
	}
	order, err = client.WaitOrder(ctx, order.URI)
	if err != nil {
		return err
	}
	certificateKey, err := loadCertificateKey(keyPath)
	if err != nil {
		return err
	}
	csr, err := x509.CreateCertificateRequest(rand.Reader, &x509.CertificateRequest{DNSNames: []string{m.host}}, certificateKey)
	if err != nil {
		return err
	}
	chain, _, err := client.CreateOrderCert(ctx, order.FinalizeURL, csr, true)
	if err != nil {
		return err
	}
	var data []byte
	for _, der := range chain {
		data = append(data, pem.EncodeToMemory(&pem.Block{Type: "CERTIFICATE", Bytes: der})...)
	}
	leaf, err := x509.ParseCertificate(chain[0])
	if err != nil || leaf.VerifyHostname(m.host) != nil {
		return fmt.Errorf("ACME certificate hostname mismatch")
	}
	if err = m.reportCertificate(ctx, certificateFingerprint(leaf.Raw)); err != nil {
		return err
	}
	tmp, err := os.CreateTemp(path, ".certificate-*")
	if err != nil {
		return err
	}
	defer os.Remove(tmp.Name())
	_, err = tmp.Write(data)
	closeErr := tmp.Close()
	if err != nil {
		return err
	}
	if closeErr != nil {
		return closeErr
	}
	if err = os.Rename(tmp.Name(), certPath); err != nil {
		return err
	}
	c, err := tls.LoadX509KeyPair(certPath, keyPath)
	if err != nil {
		return err
	}
	m.mu.Lock()
	m.certificate = &c
	m.mu.Unlock()
	return nil
}
func loadCertificateKey(path string) (*ecdsa.PrivateKey, error) {
	data, err := os.ReadFile(path)
	if err == nil {
		block, _ := pem.Decode(data)
		if block == nil {
			return nil, fmt.Errorf("invalid certificate key")
		}
		return x509.ParseECPrivateKey(block.Bytes)
	}
	if !os.IsNotExist(err) {
		return nil, err
	}
	key, err := ecdsa.GenerateKey(elliptic.P256(), rand.Reader)
	if err != nil {
		return nil, err
	}
	der, err := x509.MarshalECPrivateKey(key)
	if err != nil {
		return nil, err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0600)
	if err != nil {
		return nil, err
	}
	_, err = file.Write(pem.EncodeToMemory(&pem.Block{Type: "EC PRIVATE KEY", Bytes: der}))
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	return key, err
}
func (m *certificateManager) publishTXT(ctx context.Context, value string) error {
	p, err := relay.NewProof(ctx, m.client(), m.identity.Endpoint, m.identity.Key, m.identity.Credential, "dns:"+value)
	if err != nil {
		return err
	}
	return relayClientRequest(ctx, m.client(), m.identity.Endpoint+"/dns", map[string]any{"proof": p, "value": value})
}
func relayRequest(ctx context.Context, url string, value any) error {
	return relayClientRequest(ctx, http.DefaultClient, url, value)
}
func relayClientRequest(ctx context.Context, client *http.Client, url string, value any) error {
	raw, err := json.Marshal(value)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, "POST", url, bytes.NewReader(raw))
	if err != nil {
		return err
	}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 204 {
		return fmt.Errorf("relay control request refused (%d)", resp.StatusCode)
	}
	return nil
}

func (m *certificateManager) reportCertificate(ctx context.Context, fingerprint string) error {
	p, err := relay.NewProof(ctx, m.client(), m.identity.Endpoint, m.identity.Key, m.identity.Credential, "certificate:"+fingerprint)
	if err != nil {
		return err
	}
	return relayClientRequest(ctx, m.client(), m.identity.Endpoint+"/certificate", map[string]any{"proof": p, "fingerprint": fingerprint})
}

func certificateFingerprint(der []byte) string {
	sum := sha256.Sum256(der)
	return hex.EncodeToString(sum[:])
}
