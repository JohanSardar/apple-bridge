package scenarios

import (
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	relay "github.com/JohanSardar/apple-bridge/lab/relayfixture"
)

func runRelayContainment(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), GitSHA: strings.TrimSpace(string(sha)), RunID: "relay-r3-" + strings.ToLower(s.ID), Hosts: []string{"build"}, NetworkPath: "synthetic loopback only"}
	e.Impl = "go"
	if os.Getenv("AB_RELAY_FIXTURE") != "" {
		e.Impl = "rust"
	}
	add := func(n string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: n, Passed: ok}) }
	if level != "sim" {
		add("sim_only", false)
		return e
	}
	dir, err := os.MkdirTemp("", "relay-containment-")
	if err != nil {
		add("fixture", false)
		return e
	}
	defer os.RemoveAll(dir)
	defer relay.Cleanup(dir)
	if s.ID == "RELAY-TLS-SPLICE" || s.ID == "RELAY-DOWNGRADE" {
		relayTLSSplice(ctx, dir, add)
	} else if s.ID == "RELAY-LEGACY-COMPAT" {
		relayLegacyCompat(ctx, dir, add)
	} else {
		relayRotateGated(dir, add)
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
func relayRotateGated(dir string, add func(string, bool)) {
	limits := relay.Limits{Connections: 2, BytesPerSecond: 1048576, MonthlyBytes: 10485760}
	path := filepath.Join(dir, "state.json")
	config := filepath.Join(dir, "owner-keys")
	public, owner, _ := ed25519.GenerateKey(rand.Reader)
	_, unknown, _ := ed25519.GenerateKey(rand.Reader)
	os.WriteFile(config, []byte(base64.RawURLEncoding.EncodeToString(public)), 0600)
	store, err := relay.Open(path, "relay.test", limits)
	if err != nil {
		add("store", false)
		return
	}
	store.RequireOwnerKeys(config)
	api := httptest.NewServer(relay.New(store).Handler(""))
	defer func() { api.Close() }()
	enroll := func(key ed25519.PrivateKey, endpoint string) (relay.Credential, int) {
		response, err := http.Get(api.URL + "/challenge")
		if err != nil {
			return relay.Credential{}, 0
		}
		var c struct {
			Nonce string `json:"nonce"`
		}
		json.NewDecoder(response.Body).Decode(&c)
		response.Body.Close()
		in := relay.Enrollment{PublicKey: base64.RawURLEncoding.EncodeToString(key.Public().(ed25519.PublicKey)), Nonce: c.Nonce, Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, relay.EnrollmentMessage("relay.test", c.Nonce)))}
		var out relay.Credential
		code := relayPost(http.DefaultClient, api.URL+endpoint, in, &out, "")
		return out, code
	}
	for _, endpoint := range []string{"/enroll", "/rotate"} {
		_, code := enroll(unknown, endpoint)
		add("unknown_refused"+endpoint, code == 403)
	}
	a, code := enroll(owner, "/enroll")
	add("owner_enroll", code == 200)
	b, code := enroll(owner, "/rotate")
	add("owner_rotate", code == 200 && a.Hostname == b.Hostname && a.Credential != b.Credential)
	before, _ := store.Device(a.Hostname)
	store.Policy(a.Hostname, limits, true, false)
	disabledCredential, code := enroll(owner, "/rotate")
	disabled, _ := store.Device(a.Hostname)
	add("disabled_preserved", code == 200 && disabled.Disabled && disabled.Bytes == before.Bytes && disabled.Month == before.Month && disabled.Limits == before.Limits)
	api.Close()
	store, err = relay.Open(path, "relay.test", limits)
	if err != nil {
		add("restart", false)
		return
	}
	store.RequireOwnerKeys(config)
	api = httptest.NewServer(relay.New(store).Handler(""))
	_, code = enroll(unknown, "/rotate")
	add("restart_gate", code == 403)
	d, _ := store.Device(a.Hostname)
	add("restart_disabled", d.Disabled)
	_, attachErr := relay.New(store).Attach(a.Hostname, disabledCredential.Credential, relayTestTunnel{wire: &relayWire{}})
	add("disabled_attach_refused", attachErr != nil)
	os.Remove(config)
	frozen, _ := os.ReadFile(path)
	for _, endpoint := range []string{"/enroll", "/rotate"} {
		_, code = enroll(owner, endpoint)
		add("missing_config_closed"+endpoint, code == 403)
	}
	os.WriteFile(config, []byte("invalid"), 0600)
	_, code = enroll(owner, "/rotate")
	add("invalid_config_closed", code == 403)
	after, _ := os.ReadFile(path)
	add("denial_preserves_state", string(frozen) == string(after))
	os.WriteFile(config, []byte(base64.RawURLEncoding.EncodeToString(public)), 0600)
	store.Policy(a.Hostname, limits, false, true)
	_, code = enroll(owner, "/rotate")
	add("permanent_revocation", code == 403)
	rollback, err := relay.Open(path, "relay.test", limits)
	add("legacy_state_readable", err == nil)
	if err == nil {
		d, _ := rollback.Device(a.Hostname)
		add("rollback_revoked", d.Revoked)
	}
	api.Close()
	api = httptest.NewServer(relay.New(store).Handler(""))
	status := 0
	for i := 0; i < 31; i++ {
		r, _ := http.Get(api.URL + "/challenge")
		status = r.StatusCode
		r.Body.Close()
	}
	add("challenge_throttle", status == 429)
	for i := 0; i < 21; i++ {
		status = relayPost(http.DefaultClient, api.URL+"/rotate", relay.Enrollment{}, nil, "")
	}
	add("shared_enrollment_throttle", status == 429)
}
