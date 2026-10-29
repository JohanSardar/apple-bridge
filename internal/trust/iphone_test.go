package trust

import (
	"crypto/x509"
	"testing"
	"time"
)

func TestIPhoneCodeCannotEnrollReplica(t *testing.T) {
	t.Setenv("APPLE_BRIDGE_SECRET_STORE", "file")
	a, err := Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	for _, remote := range []bool{false, true} {
		raw, err := a.PairIPhoneCode("https://example.test", remote, time.Minute)
		if err != nil {
			t.Fatal(err)
		}
		code, err := DecodePairCode(raw)
		if err != nil {
			t.Fatal(err)
		}
		cert, err := x509.ParseCertificate(a.Certificate.Certificate[0])
		if err != nil {
			t.Fatal(err)
		}
		_, err = a.acceptPair(JoinRequest{ID: code.ID, Proof: PairProof(code.Secret, cert.Raw)}, cert, false)
		if err == nil {
			t.Fatal("phone code enrolled replica")
		}
		d, err := a.load()
		if err != nil {
			t.Fatal(err)
		}
		if len(d.Pairings[code.ID].Grants) != 0 || d.Pairings[code.ID].Purpose != "iphone" || d.Pairings[code.ID].Used {
			t.Fatal("phone code scope or purpose changed")
		}
	}
}
