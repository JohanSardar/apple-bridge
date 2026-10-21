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
	LocalLevel     AgentLevel        `json:"local_level,omitempty"`
	Challenge      string            `json:"challenge,omitempty"`
	ReplicaID      string            `json:"replica_id"`
	LocalPrincipal string            `json:"local_principal"`
	LocalGrants    []contracts.Grant `json:"local_grants"`
	CommandID      string            `json:"command_id"`
	RequestHash    string            `json:"request_hash"`
	IAT            int64             `json:"iat"`
}

func (a *Authority) SignDelegation(p Principal, id, hash string) (string, error) {
	return a.signDelegation(p, id, hash, "")
}
func (a *Authority) SignDelegationChallenge(p Principal, id, hash, challenge string) (string, error) {
	if !contracts.ValidDigest(challenge) {
		return "", contracts.ErrInvalid
	}
	return a.signDelegation(p, id, hash, challenge)
}
func (a *Authority) signDelegation(p Principal, id, hash, challenge string) (string, error) {
	e, err := a.Enrollment()
	if err != nil {
		return "", err
	}
	if e == nil {
		return "", contracts.ErrUnavailable
	}
	var level AgentLevel
	if p.Kind == "agent" {
		if p.Level != AgentRead && p.Level != AgentApprove && p.Level != AgentFull {
			return "", contracts.ErrUnauthorized
		}
		level = p.Level
	}
	b, err := json.Marshal(Delegation{ReplicaID: e.ReplicaID, LocalPrincipal: p.ID, LocalGrants: p.Grants, CommandID: id, RequestHash: hash, IAT: time.Now().Unix(), LocalLevel: level, Challenge: challenge})
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
	return a.verifyDelegation(cert, header, id, hash, "")
}
func (a *Authority) VerifyDelegationChallenge(cert *x509.Certificate, header, id, hash, challenge string) (Principal, error) {
	if !contracts.ValidDigest(challenge) {
		return Principal{}, contracts.ErrUnauthorized
	}
	return a.verifyDelegation(cert, header, id, hash, challenge)
}
func (a *Authority) verifyDelegation(cert *x509.Certificate, header, id, hash, challenge string) (Principal, error) {
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
	if contracts.DecodeInput(b, &d) != nil {
		return empty, contracts.ErrUnauthorized
	}
	agent := IsAgentPrincipalID(d.LocalPrincipal) && !strings.Contains(d.LocalPrincipal, "/") && (d.LocalLevel == AgentRead || d.LocalLevel == AgentApprove || d.LocalLevel == AgentFull)
	ordinary := (strings.HasPrefix(d.LocalPrincipal, "token:") || strings.HasPrefix(d.LocalPrincipal, "device:")) && d.LocalLevel == ""
	if d.ReplicaID != replica.ID || !agent && !ordinary || d.Challenge != challenge || d.CommandID != id || d.RequestHash != hash || time.Now().Unix()-d.IAT > 60 || d.IAT > time.Now().Unix()+5 || !Subset(d.LocalGrants, replica.Grants) {
		return empty, contracts.ErrUnauthorized
	}
	return Principal{ID: replica.ID + "/" + d.LocalPrincipal, Kind: "delegated", Level: d.LocalLevel, Grants: d.LocalGrants, Fingerprint: replica.Fingerprint}, nil
}
