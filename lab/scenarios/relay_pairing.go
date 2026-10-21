package scenarios

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"errors"
	"net/http/httptest"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/replication"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runRelayPairing(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), RunID: contracts.NewID("relay-pair"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, "sim", 0, 0)
	add("pairing_world", err == nil)
	if err != nil {
		return e
	}
	defer w.Close()
	authority, err := trust.Open(w.Gateway.Home)
	add("gateway_identity", err == nil)
	if err != nil {
		return e
	}
	feed := &replication.Server{Authority: authority}
	fixture := httptest.NewTLSServer(feed.RelayHandler())
	defer fixture.Close()
	encoded, err := authority.PairRemote(fixture.URL, []contracts.Grant{"reminders:write", "system:read"}, 10*time.Minute)
	add("remote_pair_code_created", err == nil)
	if err != nil {
		return e
	}
	code, err := trust.DecodePairCode(encoded)
	raw, _ := json.Marshal(code)
	var fields map[string]any
	json.Unmarshal(raw, &fields)
	add("remote_pair_code_transport", err == nil && fields["remote"] == true && code.Address == fixture.URL && code.Fingerprint == authority.Fingerprint)
	if fields["remote"] != true || err != nil {
		return e
	}
	replica, err := trust.Open(filepath.Join(w.Dir, "relay-joined"))
	add("replica_key_created", err == nil)
	if err != nil {
		return e
	}
	join := func(candidate trust.PairCode) error {
		conn, err := relayAuthDial(ctx, fixture)
		if err != nil {
			return err
		}
		defer conn.CloseNow()
		return replica.JoinRelay(ctx, conn, candidate)
	}
	badPin := code
	badPin.Fingerprint = strings.Repeat("0", 64)
	add("join_pins_gateway_before_using_code", errors.Is(join(badPin), contracts.ErrUnauthorized))
	add("remote_signed_pairing", join(code) == nil)
	enrollment, err := replica.Enrollment()
	add("remote_enrollment_persists_public_url_and_gateway_pin", err == nil && enrollment != nil && enrollment.Remote && enrollment.Address == fixture.URL && enrollment.Fingerprint == authority.Fingerprint && enrollment.PairingID == code.ID)
	if enrollment == nil {
		return e
	}
	cert, err := x509.ParseCertificate(replica.Certificate.Certificate[0])
	if err != nil {
		add("paired_certificate", false)
		return e
	}
	paired, err := authority.AuthenticateCertificate(cert)
	add("paired_replica_identity_and_delegable_scope", err == nil && paired.ID == enrollment.ReplicaID && paired.Fingerprint == replica.Fingerprint && trust.Subset(enrollment.Grants, []contracts.Grant{"reminders:write", "system:read"}) && trust.Subset([]contracts.Grant{"reminders:write", "system:read"}, enrollment.Grants))
	add("remote_pair_code_single_use", join(code) != nil)
	unchanged, err := replica.Enrollment()
	add("failed_replay_preserves_enrollment", err == nil && unchanged != nil && unchanged.ReplicaID == enrollment.ReplicaID)
	expired := code
	expired.ExpiresAt = time.Now().Add(-time.Second)
	add("expired_remote_code_rejected", errors.Is(join(expired), contracts.ErrExpired))
	for _, address := range []string{"http://bridge.example.com", "wss://bridge.example.com", "https://owner:password@bridge.example.com", "https://bridge.example.com/feed", "https://bridge.example.com?token=invalid", "https://bridge.example.com#fragment", "https://bridge.example.com:0", "https://bridge.example.com:65536"} {
		_, invalid := authority.PairRemote(address, []contracts.Grant{"system:read"}, time.Minute)
		add("remote_pair_url_rejected_"+address, errors.Is(invalid, contracts.ErrInvalid))
	}
	encoded, err = authority.PairRemote(fixture.URL, []contracts.Grant{"system:read"}, time.Minute)
	if err != nil {
		add("attempt_code", false)
		return e
	}
	attempt, err := trust.DecodePairCode(encoded)
	if err != nil {
		add("attempt_decode", false)
		return e
	}
	bad := attempt
	bad.Secret = append([]byte(nil), attempt.Secret...)
	bad.Secret[0] ^= 1
	for i := 0; i < 5; i++ {
		add("bad_remote_pair_secret_rejected", join(bad) != nil)
	}
	add("remote_pair_five_attempt_limit", join(attempt) != nil)

	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
