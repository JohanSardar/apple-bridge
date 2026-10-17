package trust

import (
	"crypto/ed25519"
	"crypto/x509"
	"encoding/base64"
	"encoding/json"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Delegation struct {
	ReplicaID      string            `json:"replica_id"`
	LocalPrincipal string            `json:"local_principal"`
	LocalGrants    []contracts.Grant `json:"local_grants"`
	CommandID      string            `json:"command_id"`
	RequestHash    string            `json:"request_hash"`
	IAT            int64             `json:"iat"`
}

func (a *Authority) SignDelegation(p Principal, id, hash string) (string, error) {
	e, err := a.Enrollment()
	if err != nil {
		return "", err
	}
	if e == nil {
		return "", contracts.ErrUnavailable
	}
	b, err := json.Marshal(Delegation{e.ReplicaID, p.ID, p.Grants, id, hash, time.Now().Unix()})
	if err != nil {
		return "", err
	}
	key, ok := a.Certificate.PrivateKey.(ed25519.PrivateKey)
	if !ok {
		return "", contracts.ErrCorrupt
	}
	sig := ed25519.Sign(key, b)
	return base64.RawURLEncoding.EncodeToString(b) + "." + base64.RawURLEncoding.EncodeToString(sig), nil
}
func (a *Authority) VerifyDelegation(cert *x509.Certificate, header, id, hash string) (Principal, error) {
	var empty Principal
	replica, err := a.AuthenticateCertificate(cert)
	if err != nil {
		return empty, err
	}
	parts := strings.Split(header, ".")
	if len(parts) != 2 {
		return empty, contracts.ErrUnauthorized
	}
	b, err := base64.RawURLEncoding.DecodeString(parts[0])
	if err != nil {
		return empty, contracts.ErrUnauthorized
	}
	sig, err := base64.RawURLEncoding.DecodeString(parts[1])
	if err != nil {
		return empty, contracts.ErrUnauthorized
	}
	pub, ok := cert.PublicKey.(ed25519.PublicKey)
	if !ok || !ed25519.Verify(pub, b, sig) {
		return empty, contracts.ErrUnauthorized
	}
	var d Delegation
	if contracts.DecodeInput(b, &d) != nil || d.ReplicaID != replica.ID || !strings.HasPrefix(d.LocalPrincipal, "token:") || d.CommandID != id || d.RequestHash != hash || time.Now().Unix()-d.IAT > 60 || d.IAT > time.Now().Unix()+5 || !Subset(d.LocalGrants, replica.Grants) {
		return empty, contracts.ErrUnauthorized
	}
	return Principal{ID: replica.ID + "/" + d.LocalPrincipal, Kind: "delegated", Grants: d.LocalGrants, Fingerprint: replica.Fingerprint}, nil
}
