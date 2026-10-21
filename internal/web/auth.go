package web

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"io"
	"net/http"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/go-webauthn/webauthn/protocol"
	"github.com/go-webauthn/webauthn/webauthn"
)

type owner struct{ Credentials []webauthn.Credential }

func (u owner) WebAuthnID() []byte                         { return []byte("apple-bridge-owner") }
func (u owner) WebAuthnName() string                       { return "Apple Bridge owner" }
func (u owner) WebAuthnDisplayName() string                { return "Apple Bridge owner" }
func (u owner) WebAuthnCredentials() []webauthn.Credential { return u.Credentials }

type ceremony struct {
	Data      webauthn.SessionData
	Origin    string
	Flow      string
	Kind      string
	Enroll    string
	Principal *trust.Principal
	Expires   time.Time
}

type authInput struct {
	Token      string          `json:"token"`
	Kind       string          `json:"kind"`
	CeremonyID string          `json:"ceremony_id"`
	Credential json.RawMessage `json:"credential"`
}

func (s *server) webAuthn(r *http.Request) (*webauthn.WebAuthn, owner, error) {
	keys, err := s.deps.Authority.Passkeys(rpID(r))
	if err != nil {
		return nil, owner{}, err
	}
	u := owner{Credentials: []webauthn.Credential{}}
	for _, k := range keys {
		u.Credentials = append(u.Credentials, k.Credential)
	}
	w, err := webauthn.New(&webauthn.Config{RPID: rpID(r), RPDisplayName: "Apple Bridge", RPOrigins: []string{servingOrigin(r)}, AuthenticatorSelection: protocol.AuthenticatorSelection{ResidentKey: protocol.ResidentKeyRequirementRequired, UserVerification: protocol.VerificationRequired}})
	return w, u, err
}
func (s *server) authFailure(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	key := "auth:" + clientIP(r)
	entry := s.rates[key]
	now := s.rateTime()
	if now.Sub(entry.FailureAt) >= 15*time.Minute {
		entry.Failures = 0
	}
	entry.Failures++
	entry.FailureAt = now
	if entry.Failures >= 10 {
		entry.RetryAt = now.Add(time.Duration(min(entry.Failures-9, 60)) * time.Second)
	}
	s.rates[key] = entry
	s.mu.Unlock()
	reject(w, 401, "unauthorized")
}
func (s *server) authOptions(w http.ResponseWriter, r *http.Request, flow string) {
	var input authInput
	if json.NewDecoder(r.Body).Decode(&input) != nil {
		reject(w, 400, "invalid")
		return
	}
	if input.Kind == "" {
		input.Kind = "trusted"
	}
	if input.Kind != "trusted" && input.Kind != "public" {
		reject(w, 400, "invalid")
		return
	}
	c := ceremony{Origin: servingOrigin(r), Flow: flow, Kind: input.Kind, Enroll: input.Token, Expires: time.Now().Add(5 * time.Minute)}
	if flow == "enroll" && s.deps.Authority.CheckWebEnroll(input.Token) != nil {
		s.authFailure(w, r)
		return
	}
	if flow == "passkeys" {
		p := current(r).Principal
		c.Principal = &p
	}
	wa, user, err := s.webAuthn(r)
	if err != nil {
		reject(w, 503, "unavailable")
		return
	}
	var options any
	var data *webauthn.SessionData
	if flow == "login" {
		options, data, err = wa.BeginDiscoverableLogin(webauthn.WithUserVerification(protocol.VerificationRequired))
	} else {
		options, data, err = wa.BeginRegistration(user, webauthn.WithResidentKeyRequirement(protocol.ResidentKeyRequirementRequired))
	}
	if err != nil {
		reject(w, 400, "invalid")
		return
	}
	c.Data = *data
	id := contracts.NewID("ceremony")
	s.mu.Lock()
	for key, old := range s.ceremonies {
		if time.Now().After(old.Expires) {
			delete(s.ceremonies, key)
		}
	}
	if len(s.ceremonies) >= 1024 {
		s.mu.Unlock()
		reject(w, 429, "rate_limited")
		return
	}
	s.ceremonies[id] = c
	s.mu.Unlock()
	raw, _ := json.Marshal(options)
	result := map[string]any{}
	json.Unmarshal(raw, &result)
	result["ceremony_id"] = id
	respond(w, 200, result)
}
func (s *server) authFinish(w http.ResponseWriter, r *http.Request, flow string) {
	var input authInput
	if json.NewDecoder(r.Body).Decode(&input) != nil || len(input.Credential) == 0 {
		reject(w, 400, "invalid")
		return
	}
	s.mu.Lock()
	c, ok := s.ceremonies[input.CeremonyID]
	delete(s.ceremonies, input.CeremonyID)
	s.mu.Unlock()
	if !ok || time.Now().After(c.Expires) || c.Flow != flow || c.Origin != servingOrigin(r) || flow == "passkeys" && (c.Principal == nil || c.Principal.ID != current(r).Principal.ID) {
		s.authFailure(w, r)
		return
	}
	wa, user, err := s.webAuthn(r)
	if err != nil {
		reject(w, 503, "unavailable")
		return
	}
	credentialRequest := r.Clone(r.Context())
	credentialRequest.Body = io.NopCloser(bytes.NewReader(input.Credential))
	var key *webauthn.Credential
	if flow == "login" {
		key, err = wa.FinishDiscoverableLogin(func(rawID, handle []byte) (webauthn.User, error) {
			if !bytes.Equal(handle, user.WebAuthnID()) {
				return nil, contracts.ErrUnauthorized
			}
			for _, k := range user.Credentials {
				if bytes.Equal(k.ID, rawID) {
					return user, nil
				}
			}
			return nil, contracts.ErrUnauthorized
		}, c.Data, credentialRequest)
		if err == nil && key.Authenticator.CloneWarning {
			err = contracts.ErrUnauthorized
		}
		if err == nil {
			err = s.deps.Authority.UpdatePasskey(rpID(r), *key)
		}
	} else {
		key, err = wa.FinishRegistration(user, c.Data, credentialRequest)
		if err == nil {
			err = s.deps.Authority.SavePasskey(trust.Passkey{ID: base64.RawURLEncoding.EncodeToString(key.ID), RPID: rpID(r), Credential: *key, CreatedAt: time.Now().UTC()}, c.Enroll, c.Principal)
		}
	}
	if err != nil {
		s.authFailure(w, r)
		return
	}
	s.mu.Lock()
	entry := s.rates["auth:"+clientIP(r)]
	entry.Failures = 0
	entry.RetryAt = time.Time{}
	entry.FailureAt = time.Time{}
	s.rates["auth:"+clientIP(r)] = entry
	s.mu.Unlock()
	if flow == "passkeys" {
		respond(w, 200, map[string]string{"id": base64.RawURLEncoding.EncodeToString(key.ID)})
		return
	}
	token, session, err := s.deps.Authority.CreateDeviceSession(c.Kind)
	if err != nil {
		reject(w, 503, "unavailable")
		return
	}
	setCookie(w, r, token, session, 0)
	s.broadcast(map[string]any{"t": "signin", "device": session.DeviceID, "at": time.Now().UTC()}, session.DeviceID)
	respond(w, 200, session)
}
