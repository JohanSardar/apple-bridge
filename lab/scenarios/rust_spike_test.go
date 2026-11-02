package scenarios

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"

	"bufio"
	"github.com/go-webauthn/webauthn/protocol"
	"github.com/go-webauthn/webauthn/webauthn"
	"github.com/modelcontextprotocol/go-sdk/mcp"
	"io"
	"net/http"
	"net/http/httptest"
)

func TestRustWebAuthnSpike(t *testing.T) {
	binary := os.Getenv("AB_SPIKE_BINARY")
	if binary == "" {
		t.Skip("AB_SPIKE_BINARY is unset")
	}
	invoke := func(input any) map[string]json.RawMessage {
		t.Helper()
		b, err := json.Marshal(input)
		if err != nil {
			t.Fatal(err)
		}
		cmd := exec.Command(binary)
		cmd.Stdin = bytes.NewReader(b)
		out, err := cmd.CombinedOutput()
		if err != nil {
			t.Fatalf("Rust: %v %s", err, out)
		}
		var result map[string]json.RawMessage
		if err := json.Unmarshal(out, &result); err != nil {
			t.Fatal(err)
		}
		return result
	}
	fixture, err := os.ReadFile("../../contracts/golden/passkey.json")
	if err != nil {
		t.Fatal(err)
	}
	result := invoke(map[string]any{"op": "golden", "fixture": json.RawMessage(fixture)})
	t.Logf("Go registered -> Rust authenticated: %s", result["result"])
	wa, err := webauthn.New(&webauthn.Config{RPID: "localhost", RPDisplayName: "Bridge", RPOrigins: []string{"http://localhost:7780"}, AuthenticatorSelection: protocol.AuthenticatorSelection{ResidentKey: "required", UserVerification: "required"}})
	if err != nil {
		t.Fatal(err)
	}
	auth, err := newWebAuthenticator()
	if err != nil {
		t.Fatal(err)
	}
	started := invoke(map[string]any{"op": "start"})
	var options struct {
		PublicKey struct {
			Challenge              string
			User                   struct{ ID string }
			AuthenticatorSelection struct {
				ResidentKey      string
				UserVerification string
			}
		}
	}
	if err := json.Unmarshal(started["options"], &options); err != nil {
		t.Fatal(err)
	}
	if options.PublicKey.User.ID != base64.RawURLEncoding.EncodeToString([]byte("apple-bridge-owner")) || options.PublicKey.AuthenticatorSelection.ResidentKey != "required" || options.PublicKey.AuthenticatorSelection.UserVerification != "required" {
		t.Fatalf("invalid options: %s", started["options"])
	}
	response, err := auth.credential(true, options.PublicKey.Challenge, "http://localhost:7780", "localhost")
	if err != nil {
		t.Fatal(err)
	}
	registered := invoke(map[string]any{"op": "finish", "state": started["state"], "response": response})
	var cred struct {
		ID             string `json:"cred_id"`
		Counter        uint32 `json:"counter"`
		Verified       bool   `json:"user_verified"`
		BackupEligible bool   `json:"backup_eligible"`
		BackupState    bool   `json:"backup_state"`
	}
	encoded, _ := json.Marshal(registered)
	if err := json.Unmarshal(encoded, &cred); err != nil {
		t.Fatal(err)
	}

	id, err := base64.RawURLEncoding.DecodeString(cred.ID)
	if err != nil {
		t.Fatal(err)
	}
	var key map[string]any
	if err := json.Unmarshal(registered["cred"], &key); err != nil {
		t.Fatal(err)
	}
	keyJSON, _ := json.Marshal(key)
	var cose struct {
		Key struct {
			EC_EC2 struct {
				X string
				Y string
			}
		}
	}
	if err := json.Unmarshal(keyJSON, &cose); err != nil {
		t.Fatal(err)
	}
	x, err := base64.RawURLEncoding.DecodeString(cose.Key.EC_EC2.X)
	if err != nil {
		t.Fatal(err)
	}
	y, err := base64.RawURLEncoding.DecodeString(cose.Key.EC_EC2.Y)
	if err != nil {
		t.Fatal(err)
	}
	owner := goldenOwner{keys: []webauthn.Credential{{ID: id, PublicKey: webCBOR(map[int]any{1: 2, 3: -7, -1: 1, -2: x, -3: y}), AttestationType: "none", AttestationFormat: "none", Transport: []protocol.AuthenticatorTransport{"internal"}, Flags: webauthn.CredentialFlags{UserPresent: true, UserVerified: cred.Verified, BackupEligible: cred.BackupEligible, BackupState: cred.BackupState}, Authenticator: webauthn.Authenticator{SignCount: cred.Counter}}}}
	for _, verified := range []bool{true, false} {
		auth.UserVerified = verified
		_, session, err := wa.BeginDiscoverableLogin(webauthn.WithUserVerification("required"))
		if err != nil {
			t.Fatal(err)
		}
		response, err := auth.credential(false, session.Challenge, "http://localhost:7780", "localhost")
		if err != nil {
			t.Fatal(err)
		}
		b, _ := json.Marshal(response)
		_, err = wa.FinishDiscoverableLogin(func(id, handle []byte) (webauthn.User, error) {
			if !bytes.Equal(id, owner.keys[0].ID) || !bytes.Equal(handle, owner.WebAuthnID()) {
				return nil, fmt.Errorf("id/handle mismatch")
			}
			return owner, nil
		}, *session, httptest.NewRequest("POST", "http://localhost:7780/auth/login/finish", bytes.NewReader(b)))
		if verified && err != nil {
			t.Fatal(err)
		}
		if !verified && err == nil {
			t.Fatal("Go accepted missing UV")
		}
	}
	for _, verified := range []bool{true, false} {
		auth.UserVerified = verified
		started := invoke(map[string]any{"op": "auth-start", "credential": registered})
		var options struct{ PublicKey struct{ Challenge string } }
		if err := json.Unmarshal(started["options"], &options); err != nil {
			t.Fatal(err)
		}
		response, err := auth.credential(false, options.PublicKey.Challenge, "http://localhost:7780", "localhost")
		if err != nil {
			t.Fatal(err)
		}
		finished := invoke(map[string]any{"op": "auth-finish", "state": started["state"], "response": response})
		if string(finished["accepted"]) != fmt.Sprint(verified) {
			t.Fatalf("Rust UV %v: %s", verified, finished["result"])
		}
	}
	t.Log("Rust registered -> Go authenticated; missing UV rejected; no re-enrolment")
}

func TestRustMCPSpike(t *testing.T) {
	binary := os.Getenv("AB_SPIKE_BINARY")
	if binary == "" {
		t.Skip("AB_SPIKE_BINARY is unset")
	}
	root, err := filepath.Abs("../..")
	if err != nil {
		t.Fatal(err)
	}
	for _, mode := range []string{"json", "sse"} {
		t.Run(mode, func(t *testing.T) {
			cmd := exec.Command(binary, "serve", mode)
			cmd.Dir = root
			out, err := cmd.StdoutPipe()
			if err != nil {
				t.Fatal(err)
			}
			if err := cmd.Start(); err != nil {
				t.Fatal(err)
			}
			defer func() { _ = cmd.Process.Kill(); _ = cmd.Wait() }()
			scanner := bufio.NewScanner(out)
			if !scanner.Scan() {
				t.Fatal("no address")
			}
			ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
			defer cancel()
			client := mcp.NewClient(&mcp.Implementation{Name: "synthetic", Version: "1"}, nil)
			session, err := client.Connect(ctx, &mcp.StreamableClientTransport{Endpoint: "http://" + scanner.Text() + "/mcp"}, nil)
			if err != nil {
				t.Fatal(err)
			}
			defer session.Close()
			tools, err := session.ListTools(ctx, nil)
			if err != nil || len(tools.Tools) == 0 {
				t.Fatalf("tools: %v %v", tools, err)
			}
			_, err = session.CallTool(ctx, &mcp.CallToolParams{Name: "records_query", Arguments: map[string]any{"collection": "contacts.contact"}})
			if err == nil {
				t.Fatal("expected unauthorized")
			}
			t.Logf("Go client: %d tools, error %v", len(tools.Tools), err)
			short, stop := context.WithCancel(ctx)
			defer stop()
			completed := make(chan error, 1)
			go func() {
				_, err := session.CallTool(short, &mcp.CallToolParams{Name: "spike_wait"})
				completed <- err
			}()
			active := func() string {
				response, err := http.Get("http://" + scanner.Text() + "/active")
				if err != nil {
					t.Fatal(err)
				}
				body, err := io.ReadAll(response.Body)
				response.Body.Close()
				if err != nil {
					t.Fatal(err)
				}
				return string(body)
			}
			deadline := time.Now().Add(2 * time.Second)
			for active() != "1" {
				if time.Now().After(deadline) {
					t.Fatal("handler never started")
				}
				time.Sleep(time.Millisecond)
			}
			stop()
			if err := <-completed; err == nil {
				t.Fatal("missing cancellation error")
			}
			deadline = time.Now().Add(2 * time.Second)
			for active() != "0" {
				if time.Now().After(deadline) {
					t.Fatal("cancelled handler remains active")
				}
				time.Sleep(time.Millisecond)
			}
			t.Log("cancelled in-flight call: active handlers returned to zero")

		})
	}
}
