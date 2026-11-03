//go:build lab

package trust

import "github.com/JohanSardar/apple-bridge/internal/contracts"

func (a *Authority) LabPrincipalByFingerprint(fingerprint string) (Principal, error) {
	d, err := a.load()
	if err != nil {
		return Principal{}, err
	}
	for _, p := range d.Principals {
		if p.Fingerprint == fingerprint && principalActive(p) {
			return p, nil
		}
	}
	return Principal{}, contracts.ErrUnauthorized
}
