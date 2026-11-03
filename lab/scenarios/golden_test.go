package scenarios

import (
	"bytes"
	"crypto/ecdsa"
	"crypto/elliptic"
	"encoding/base64"
	"encoding/json"
	"math/big"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/JohanSardar/apple-bridge/contracts/golden"
	"github.com/go-webauthn/webauthn/protocol"
	"github.com/go-webauthn/webauthn/webauthn"
)

type goldenOwner struct{ keys []webauthn.Credential }

func (u goldenOwner) WebAuthnID() []byte                         { return []byte("apple-bridge-owner") }
func (u goldenOwner) WebAuthnName() string                       { return "Bridge owner" }
func (u goldenOwner) WebAuthnDisplayName() string                { return "Bridge owner" }
func (u goldenOwner) WebAuthnCredentials() []webauthn.Credential { return u.keys }
func TestPasskeyGolden(t *testing.T) {
	scalar := big.NewInt(42)
	x, y := elliptic.P256().ScalarBaseMult(scalar.Bytes())
	authenticator := &webAuthenticator{UserVerified: true, ID: bytes.Repeat([]byte{17}, 32), Key: &ecdsa.PrivateKey{PublicKey: ecdsa.PublicKey{Curve: elliptic.P256(), X: x, Y: y}, D: scalar}}
	wa, e := webauthn.New(&webauthn.Config{RPID: "localhost", RPDisplayName: "Bridge", AuthenticatorSelection: protocol.AuthenticatorSelection{ResidentKey: protocol.ResidentKeyRequirementRequired, UserVerification: protocol.VerificationRequired}, RPOrigins: []string{"http://localhost:7780"}})
	if e != nil {
		t.Fatal(e)
	}
	owner := goldenOwner{}
	options, session, e := wa.BeginRegistration(owner, webauthn.WithResidentKeyRequirement("required"))
	if e != nil {
		t.Fatal(e)
	}
	t.Log("ADR 0027 addendum: only pubKeyCredParams changes; ES256 and Ed25519 remove unsupported RSA without changing the signed ceremony")
	options.Response.Parameters = []protocol.CredentialParameter{{Type: protocol.PublicKeyCredentialType, Algorithm: -7}, {Type: protocol.PublicKeyCredentialType, Algorithm: -8}}
	challenge := bytes.Repeat([]byte{23}, 32)
	options.Response.Challenge = challenge
	session.Challenge = base64.RawURLEncoding.EncodeToString(challenge)
	credential, e := authenticator.credential(true, session.Challenge, "http://localhost:7780", "localhost")
	if e != nil {
		t.Fatal(e)
	}
	b, e := json.Marshal(credential)
	if e != nil {
		t.Fatal(e)
	}
	registration, e := wa.FinishRegistration(owner, *session, httptest.NewRequest("POST", "http://localhost:7780/auth/enroll/finish", bytes.NewReader(b)))
	if e != nil {
		t.Fatal(e)
	}
	owner.keys = []webauthn.Credential{*registration}
	login, session, e := wa.BeginDiscoverableLogin(webauthn.WithUserVerification("required"))
	if e != nil {
		t.Fatal(e)
	}
	login.Response.Challenge = challenge
	session.Challenge = base64.RawURLEncoding.EncodeToString(challenge)
	assertion, e := authenticator.credential(false, session.Challenge, "http://localhost:7780", "localhost")
	if e != nil {
		t.Fatal(e)
	}
	b, e = json.Marshal(assertion)
	if e != nil {
		t.Fatal(e)
	}
	_, e = wa.FinishDiscoverableLogin(func(id, handle []byte) (webauthn.User, error) {
		if !bytes.Equal(handle, owner.WebAuthnID()) {
			t.Fatal("user handle mismatch")
		}
		return owner, nil
	}, *session, httptest.NewRequest("POST", "http://localhost:7780/auth/login/finish", bytes.NewReader(b)))
	if e != nil {
		t.Fatal(e)
	}
	b, e = json.MarshalIndent(map[string]any{"impl": "go", "origin": "http://localhost:7780", "enroll_options": options, "enroll_credential": credential, "stored_credential": registration, "login_options": login, "login_credential": assertion, "user_handle": "apple-bridge-owner", "synthetic_scalar": strings.Repeat("0", 62) + "2a"}, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	golden.Check(t, "passkey.json", append(b, '\n'))
}
