package scenarios

import (
	"bytes"
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

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func runIPhoneCommand(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-iphone-command-")
	add("isolated_home", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	a, err := trust.Open(filepath.Join(dir, "trust"))
	add("authority", err == nil)
	if err != nil {
		return e
	}
	store, err := state.Open(filepath.Join(dir, "state"))
	add("state", err == nil)
	if err != nil {
		return e
	}
	defer func() { store.Close() }()
	add("ledger", store.InitCommands(ctx) == nil)
	delivery := iphone.NewDelivery(store, a)
	gateway := &operations.LocalGateway{State: store, Authority: a, Context: ctx, IPhone: delivery}
	delivery.Authorize = gateway.ReauthorizeIPhone
	delivery.Decide = gateway.DecidePhoneApproval
	mux := http.NewServeMux()
	mux.Handle("/v1/iphone/pair", a.IPhoneHandler())
	mux.Handle("/v1/iphone/", delivery.Handler())
	server := httptest.NewTLSServer(mux)
	defer server.Close()
	code, err := a.PairIPhoneCode(server.URL, false, time.Minute)
	add("pair_code", err == nil)
	phone, err := simapple.NewIPhone()
	add("phone", err == nil)
	if err != nil {
		return e
	}
	status, err := phone.Pair(server.Client(), server.URL, code)
	add("paired", err == nil && status == 200)
	token, err := a.CreateToken([]contracts.Grant{"iphone:write"})
	add("command_token", err == nil)
	principal, err := a.AuthenticateToken(token)
	add("principal", err == nil)
	args, _ := json.Marshal(contracts.IPhoneTarget{DeviceID: phone.ID})
	command := contracts.Command{ID: "cmd_iphone_roundtrip", Op: "iphone.ping", Args: args, WaitMS: 1000}
	if s.ID == "APP-NOTIFY-SIM" {
		proof := filepath.Join(dir, "notification-completion")
		build := exec.CommandContext(ctx, "xcrun", "swiftc", "-swift-version", "6", "-parse-as-library", "ios/Delivery/NotificationCompletion.swift", "lab/ios/NotificationProof.swift", "-o", proof)
		build.Env = append(os.Environ(), "SDKROOT=")
		buildErr := build.Run()
		add("notification_completion_build", buildErr == nil)
		if buildErr == nil {
			output, proofErr := exec.CommandContext(ctx, proof).Output()
			add("background_notification_completes_on_main_after_delivery", proofErr == nil && string(output) == "PASS\n")
		}
		command.Op = "iphone.notify"
		command.Args, _ = json.Marshal(contracts.IPhoneNotifyArgs{DeviceID: phone.ID, Title: "[ab-test] Notification", Body: "Bridge Test", Buttons: []contracts.IPhoneNotificationButton{{ID: "approve", Title: "Approve"}, {ID: "deny", Title: "Deny"}}, Thread: "ab-test", TimeSensitive: true})
		invalid := command
		invalid.Args = json.RawMessage(`{"device_id":"iphone:test","title":"test","body":"test","buttons":[{"id":"same","title":"A"},{"id":"same","title":"B"}]}`)
		add("duplicate_button_rejected", contracts.ValidateIPhoneCommand(invalid) != nil)
	}
	admitted, err := gateway.Submit(ctx, principal, command)
	add("offline_admitted_without_helper", err == nil && admitted.State == contracts.CommandAdmitted)
	store.Close()
	store, err = state.Open(filepath.Join(dir, "state"))
	add("restart_state", err == nil)
	if err != nil {
		return e
	}
	add("restart_ledger", store.InitCommands(ctx) == nil)
	gateway.State = store
	delivery.State = store
	add("recover_preserves_queue", gateway.Recover(ctx) == nil)
	queued, err := store.Command(ctx, command.ID)
	add("durable_offline_queue", err == nil && queued.State == contracts.CommandAdmitted)
	relayStore, err := relay.Open(filepath.Join(dir, "relay.json"), "example.test", relay.Limits{Connections: 4, BytesPerSecond: 65536, MonthlyBytes: 1048576})
	add("relay_store", err == nil)
	if err != nil {
		return e
	}
	_, key, _ := ed25519.GenerateKey(rand.Reader)
	nonce, _ := relayStore.Challenge()
	credential, err := relayStore.Enroll(relay.Enrollment{Nonce: nonce, PublicKey: base64.RawURLEncoding.EncodeToString(key.Public().(ed25519.PublicKey)), Signature: base64.RawURLEncoding.EncodeToString(ed25519.Sign(key, relay.EnrollmentMessage("example.test", nonce)))})
	add("relay_enrollment", err == nil)
	pushes := make(chan iphone.Packet, 4)
	relayServer := relay.New(relayStore)
	defer relayServer.Close()
	relayServer.IPhonePush = func(ctx context.Context, token string, raw json.RawMessage, expiry time.Time) error {
		var packet iphone.Packet
		if err := json.Unmarshal(raw, &packet); err != nil {
			return err
		}
		pushes <- packet
		return nil
	}
	relayHTTP := httptest.NewTLSServer(relayServer.Handler(""))
	defer relayHTTP.Close()
	delivery.Push = func(ctx context.Context, token string, packet iphone.Packet, expiry time.Time) error {
		raw, _ := json.Marshal(relay.IPhonePush{Hostname: credential.Hostname, Token: token, Packet: packet, Expires: expiry.Unix()})
		req, _ := http.NewRequestWithContext(ctx, "POST", relayHTTP.URL+"/iphone/push", bytes.NewReader(raw))
		req.Header.Set("Authorization", "Bearer "+credential.Credential)
		response, err := relayHTTP.Client().Do(req)
		if err != nil {
			return err
		}
		defer response.Body.Close()
		if response.StatusCode != 204 {
			return contracts.ErrUnavailable
		}
		return nil
	}
	raw, _ := json.Marshal(map[string]string{"token": strings.Repeat("a", 64)})
	response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/push-token", raw)
	add("signed_push_registration", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	runctx, cancel := context.WithCancel(ctx)
	defer cancel()
	done := make(chan struct{})
	go func() { defer close(done); delivery.Run(runctx) }()
	defer func() { cancel(); <-done }()
	var packet iphone.Packet
	select {
	case packet = <-pushes:
		add("encrypted_relay_push", true)
	case <-time.After(3 * time.Second):
		add("encrypted_relay_push", false)
		return e
	}
	plain, err := iphone.Open(phone.Encryption, phone.ID, packet, phone.Gateway)
	add("gateway_signature_and_decryption", err == nil)
	if err != nil {
		return e
	}
	tampered := packet
	tampered.Box = append([]byte(nil), packet.Box...)
	tampered.Box[0] ^= 1
	_, err = iphone.Open(phone.Encryption, phone.ID, tampered, phone.Gateway)
	add("tampered_ciphertext_rejected", err != nil)
	var job iphone.Job
	add("command_payload", json.Unmarshal(plain, &job) == nil && job.Command.ID == command.ID && job.Hash == queued.RequestHash)
	if s.ID == "APP-NOTIFY-SIM" {
		var notification contracts.IPhoneNotifyArgs
		add("notification_fields_preserved", json.Unmarshal(job.Command.Args, &notification) == nil && notification.Title == "[ab-test] Notification" && notification.Body == "Bridge Test" && notification.Thread == "ab-test" && notification.TimeSensitive && len(notification.Buttons) == 2)
	}
	result := iphone.Result{ID: command.ID, Hash: "wrong", State: contracts.CommandObserved, Data: json.RawMessage(`{"pong":true}`)}
	raw, _ = json.Marshal(result)
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/result", raw)
	add("wrong_result_hash_rejected", err == nil && response.StatusCode == 400)
	if response != nil {
		response.Body.Close()
	}
	result.Hash = job.Hash
	if s.ID == "APP-NOTIFY-SIM" {
		result.Data = json.RawMessage(`{"action_id":"approve"}`)
	}
	raw, _ = json.Marshal(result)
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/result", raw)
	add("signed_result", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	observed, err := store.Command(ctx, command.ID)
	add("ledger_observed", err == nil && observed.State == contracts.CommandObserved && len(observed.Evidence) > 0)
	if s.ID == "APP-NOTIFY-SIM" {
		var receipt struct {
			Payload []byte `json:"payload"`
		}
		var action iphone.Result
		add("button_result_returned", json.Unmarshal(observed.Evidence, &receipt) == nil && json.Unmarshal(receipt.Payload, &action) == nil && string(action.Data) == `{"action_id":"approve"}` && contracts.IPhoneCompletionEvidence(observed.Evidence, command, observed.RequestHash))
	}
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/result", raw)
	add("duplicate_result_idempotent", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	duplicate, err := gateway.Submit(ctx, principal, command)
	add("command_retry_not_redispatched", err == nil && duplicate.State == contracts.CommandObserved)
	expired := queued
	expired.Command.ID = "cmd_iphone_expired"
	expired.ExpiresAt = time.Now().Add(-time.Second)
	expired.State = contracts.CommandAdmitted
	expired.History = nil
	expired.Cursor = contracts.Cursor{}
	_, err = store.TransitionCommand(ctx, state.CommandTransition{Record: expired})
	add("expired_offline_fixture", err == nil)
	response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/pending", nil)
	add("pending_after_completion", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	expired, err = store.Command(ctx, expired.Command.ID)
	add("offline_expiry", err == nil && expired.State == contracts.CommandExpired)
	_, agentToken, err := a.CreateAgent("[ab-test] read", trust.AgentRead, []contracts.Grant{"iphone:write"})
	add("read_agent", err == nil)
	agent, err := a.AuthenticateToken(agentToken)
	add("read_agent_principal", err == nil)
	command.ID = "cmd_iphone_forbidden"
	_, err = gateway.Submit(ctx, agent, command)
	add("read_agent_cannot_dispatch", err != nil)
	_, approveToken, err := a.CreateAgent("[ab-test] approve", trust.AgentApprove, []contracts.Grant{"iphone:write"})
	add("approve_agent", err == nil)
	approver, err := a.AuthenticateToken(approveToken)
	add("approve_agent_principal", err == nil)
	command.ID = "cmd_iphone_approval"
	pending, err := gateway.Submit(ctx, approver, command)
	add("iphone_requires_owner_approval", err == nil && pending.State == contracts.CommandAwaitingApproval)
	if s.ID == "APP-APPROVE-SIM" {
		response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/pending", nil)
		var packets []iphone.Packet
		if err == nil {
			json.NewDecoder(response.Body).Decode(&packets)
			response.Body.Close()
		}
		found := false
		for _, packet := range packets {
			plain, openErr := iphone.Open(phone.Encryption, phone.ID, packet, phone.Gateway)
			var approval iphone.Job
			if openErr == nil && json.Unmarshal(plain, &approval) == nil && approval.Command.Op == "bridge.approval" && approval.Hash == pending.RequestHash {
				var args map[string]string
				found = json.Unmarshal(approval.Command.Args, &args) == nil && strings.Contains(args["body"], approver.ID) && strings.Contains(args["body"], string(command.Args))
			}
		}
		add("encrypted_approval_notification", found)
		decide := func(hash string, approve bool) int {
			payload, _ := json.Marshal(map[string]any{"command_id": command.ID, "request_hash": hash, "approve": approve})
			response, requestErr := phone.Request(server.Client(), server.URL, "/v1/iphone/approval", payload)
			if requestErr != nil {
				return 0
			}
			defer response.Body.Close()
			return response.StatusCode
		}
		add("wrong_approval_hash_rejected", decide("wrong", false) == 400)
		add("signed_phone_rejection", decide(pending.RequestHash, false) == 200)
		rejected, _ := store.Command(ctx, command.ID)
		add("ledger_rejected_by_owner", rejected.State == contracts.CommandRejectedByOwner)
		add("decision_replay_idempotent", decide(pending.RequestHash, true) == 200)
		rejected, _ = store.Command(ctx, command.ID)
		add("replay_cannot_reverse_rejection", rejected.State == contracts.CommandRejectedByOwner)
		command.ID = "cmd_phone_approve"
		pending, err = gateway.Submit(ctx, approver, command)
		add("second_approval_pending", err == nil && pending.State == contracts.CommandAwaitingApproval)
		add("signed_phone_approval", decide(pending.RequestHash, true) == 200)
		approved, _ := store.Command(ctx, command.ID)
		add("ledger_admitted_by_phone", approved.State == contracts.CommandAdmitted)
		return e
	}
	ownerToken, _, err := a.CreateDeviceSession("trusted")
	add("owner_device", err == nil)
	owner, _, err := a.AuthenticateDevice(ownerToken)
	add("owner_authenticated", err == nil)
	approved, err := gateway.DecideApproval(ctx, owner, command.ID, true)
	add("iphone_owner_approval_without_helper", err == nil && approved.State == contracts.CommandAdmitted)
	return e
}
