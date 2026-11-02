package world

import (
	"crypto/sha256"
	"crypto/tls"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"path/filepath"
	"time"
)

func identityFingerprint(data []byte) string {
	digest := sha256.Sum256(data)
	return hex.EncodeToString(digest[:])
}

func (w *World) Certificate(endpoint Endpoint) (tls.Certificate, error) {
	public, err := w.execute(endpoint.Remote, "cat", filepath.Join(endpoint.Home, "identity.pem"))
	if err != nil {
		return tls.Certificate{}, err
	}
	key, err := w.execute(endpoint.Remote, "cat", filepath.Join(endpoint.Home, "secrets", identityFingerprint([]byte("identity-key"))))
	if err != nil {
		return tls.Certificate{}, err
	}
	return tls.X509KeyPair([]byte(public), []byte(key))
}

func (w *World) FeedClient(endpoint Endpoint) (*tls.Config, string, string, error) {
	certificate, err := w.Certificate(endpoint)
	if err != nil {
		return nil, "", "", err
	}
	raw, err := w.execute(endpoint.Remote, "cat", filepath.Join(endpoint.Home, "credentials.json"))
	if err != nil {
		return nil, "", "", err
	}
	var document struct {
		Enrollment struct {
			Address     string `json:"address"`
			Fingerprint string `json:"fingerprint"`
			ReplicaID   string `json:"replica_id"`
		} `json:"enrollment"`
	}
	if err = json.Unmarshal([]byte(raw), &document); err != nil {
		return nil, "", "", err
	}
	enrollment := document.Enrollment
	if enrollment.Address == "" || enrollment.ReplicaID == "" {
		return nil, "", "", fmt.Errorf("replica enrollment missing")
	}
	config := pinnedTLS(certificate, enrollment.Fingerprint)
	return config, enrollment.Address, enrollment.ReplicaID, nil
}

func pinnedTLS(certificate tls.Certificate, fingerprint string) *tls.Config {
	return &tls.Config{MinVersion: tls.VersionTLS13, Certificates: []tls.Certificate{certificate}, InsecureSkipVerify: true, VerifyConnection: func(connection tls.ConnectionState) error {
		if len(connection.PeerCertificates) == 0 {
			return fmt.Errorf("missing peer certificate")
		}
		cert := connection.PeerCertificates[0]
		if identityFingerprint(cert.Raw) != fingerprint {
			return fmt.Errorf("peer pin mismatch")
		}
		if now := time.Now(); now.Before(cert.NotBefore) || now.After(cert.NotAfter) {
			return fmt.Errorf("peer certificate expired")
		}
		return nil
	}}
}
