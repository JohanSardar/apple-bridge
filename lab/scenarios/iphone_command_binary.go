package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"html"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/phonefixture"
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
	gateway, phone, token, err := iphoneBinaryFixture(ctx, dir)
	add("authority", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = fmt.Sprint(err)
		return e
	}
	defer gateway.Close()
	add("state", err == nil)
	add("ledger", err == nil)
	add("pair_code", err == nil)
	add("phone", err == nil)
	add("paired", err == nil)
	add("command_token", token != "")
	add("principal", token != "")
	submit := func(credential string, command contracts.Command) (contracts.CommandRecord, error) {
		var row contracts.CommandRecord
		err := gateway.Invoke(command.Op, command, credential, &row)
		return row, err
	}
	read := func(id string) (contracts.CommandRecord, error) {
		var row contracts.CommandRecord
		err := gateway.Record(id, &row)
		return row, err
	}
	args, _ := json.Marshal(contracts.IPhoneTarget{DeviceID: phone.ID})
	command := contracts.Command{ID: "cmd_iphone_roundtrip", Op: "iphone.ping", Args: args, WaitMS: 1000}
	admitted, err := submit(token, command)
	add("offline_admitted_without_helper", err == nil && admitted.State == contracts.CommandAdmitted)
	err = gateway.Restart()
	add("restart_state", err == nil)
	queued, err := read(command.ID)
	add("restart_ledger", err == nil)
	add("recover_preserves_queue", err == nil && queued.State == contracts.CommandAdmitted)
	add("durable_offline_queue", err == nil && queued.State == contracts.CommandAdmitted)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
	}
	sink, err := phonefixture.LiveSink(gateway)
	add("relay_store", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
		return e
	}
	defer sink.Close()
	add("relay_enrollment", err == nil)
	raw, _ := json.Marshal(map[string]string{"token": strings.Repeat("a", 64)})
	response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/push-token", raw)
	add("signed_push_registration", err == nil && response.StatusCode == 200)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
	} else if response.StatusCode != 200 {
		body, _ := io.ReadAll(response.Body)
		e.Assertions[len(e.Assertions)-1].Detail = fmt.Sprintf("HTTP %d %s", response.StatusCode, body)
	}
	if response != nil {
		response.Body.Close()
	}
	packet, err := sink.Next()
	add("encrypted_relay_push", err == nil)
	if err != nil {
		log, _ := os.ReadFile(filepath.Join(dir, "gateway.log"))
		e.Assertions[len(e.Assertions)-1].Detail = string(log)
	}
	if err != nil {
		return e
	}
	plain, err := phone.Open(packet)
	add("gateway_signature_and_decryption", err == nil)
	if err != nil {
		return e
	}
	tampered := packet
	tampered.Box = append([]byte(nil), packet.Box...)
	tampered.Box[0] ^= 1
	_, err = phone.Open(tampered)
	add("tampered_ciphertext_rejected", err != nil)
	var job simapple.IPhoneJob
	add("command_payload", json.Unmarshal(plain, &job) == nil && job.Command.ID == command.ID && job.Hash == queued.RequestHash)
	result := simapple.IPhoneResult{ID: command.ID, Hash: "wrong", State: contracts.CommandObserved, Data: json.RawMessage(`{"pong":true}`)}
	raw, _ = json.Marshal(result)
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/result", raw)
	add("wrong_result_hash_rejected", err == nil && response.StatusCode == 400)
	if response != nil {
		response.Body.Close()
	}
	result.Hash = job.Hash
	raw, _ = json.Marshal(result)
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/result", raw)
	add("signed_result", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	observed, err := read(command.ID)
	add("ledger_observed", err == nil && observed.State == contracts.CommandObserved && len(observed.Evidence) > 0)
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/result", raw)
	add("duplicate_result_idempotent", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	duplicate, err := submit(token, command)
	add("command_retry_not_redispatched", err == nil && duplicate.State == contracts.CommandObserved)
	expiredID := "cmd_iphone_expired"
	err = gateway.Expire(command.ID, expiredID)
	add("expired_offline_fixture", err == nil)
	response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/pending", nil)
	add("pending_after_completion", err == nil && response.StatusCode == 200)
	if response != nil {
		response.Body.Close()
	}
	expired, err := read(expiredID)
	add("offline_expiry", err == nil && expired.State == contracts.CommandExpired)
	agentToken, _, err := gateway.Agent("read")
	add("read_agent", err == nil)
	add("read_agent_principal", agentToken != "")
	command.ID = "cmd_iphone_forbidden"
	_, err = submit(agentToken, command)
	add("read_agent_cannot_dispatch", err != nil)
	approveToken, approverID, err := gateway.Agent("approve")
	add("approve_agent", err == nil)
	add("approve_agent_principal", approveToken != "")
	command.ID = "cmd_iphone_approval"
	pending, err := submit(approveToken, command)
	add("iphone_requires_owner_approval", err == nil && pending.State == contracts.CommandAwaitingApproval)
	if s.ID == "APP-APPROVE-SIM" {
		response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/pending", nil)
		var packets []simapple.IPhonePacket
		if err == nil {
			json.NewDecoder(response.Body).Decode(&packets)
			response.Body.Close()
		}
		found := false
		for _, packet := range packets {
			plain, openErr := phone.Open(packet)
			var approval simapple.IPhoneJob
			if openErr == nil && json.Unmarshal(plain, &approval) == nil && approval.Command.Op == "bridge.approval" && approval.Hash == pending.RequestHash {
				var args map[string]string
				found = json.Unmarshal(approval.Command.Args, &args) == nil && strings.Contains(args["body"], approverID) && strings.Contains(args["body"], string(command.Args))
			}
		}
		add("encrypted_approval_notification", found)
		decide := func(hash string, approve bool) int {
			payload, _ := json.Marshal(map[string]any{"command_id": command.ID, "request_hash": hash, "approve": approve})
			response, requestErr := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/approval", payload)
			if requestErr != nil {
				return 0
			}
			defer response.Body.Close()
			return response.StatusCode
		}
		add("wrong_approval_hash_rejected", decide("wrong", false) == 400)
		add("signed_phone_rejection", decide(pending.RequestHash, false) == 200)
		rejected, _ := read(command.ID)
		add("ledger_rejected_by_owner", rejected.State == contracts.CommandRejectedByOwner)
		add("decision_replay_idempotent", decide(pending.RequestHash, true) == 200)
		rejected, _ = read(command.ID)
		add("replay_cannot_reverse_rejection", rejected.State == contracts.CommandRejectedByOwner)
		command.ID = "cmd_phone_approve"
		pending, err = submit(approveToken, command)
		add("second_approval_pending", err == nil && pending.State == contracts.CommandAwaitingApproval)
		add("signed_phone_approval", decide(pending.RequestHash, true) == 200)
		var returned contracts.CommandRecord
		raw, readErr := os.ReadFile(filepath.Join(gateway.Home, "phone-decision.json"))
		if readErr != nil || json.Unmarshal(raw, &returned) != nil {
			add("phone_admission_returned", false)
			return e
		}
		approved, readErr := read(command.ID)
		admissions := 0
		for _, event := range approved.History {
			if event.State == contracts.CommandAdmitted {
				admissions++
			}
		}
		add("ledger_admitted_by_phone", readErr == nil && returned.State == contracts.CommandAdmitted && approved.RequestHash == pending.RequestHash && admissions == 1)
		return e
	}
	owner, err := phoneOwnerSession(ctx, gateway)
	add("owner_device", err == nil)
	if err != nil {
		e.Assertions[len(e.Assertions)-1].Detail = err.Error()
		return e
	}
	add("owner_authenticated", owner.cookie != nil)
	var body []byte
	response, body, err = owner.request("POST", "approvals/"+command.ID, map[string]string{"decision": "approve"})
	var approved contracts.CommandRecord
	start := strings.Index(string(body), "<pre>")
	end := strings.Index(string(body), "</pre>")
	decoded := start >= 0 && end > start && json.Unmarshal([]byte(html.UnescapeString(string(body[start+5:end]))), &approved) == nil
	add("iphone_owner_approval_without_helper", err == nil && response.StatusCode == 200 && decoded && approved.State == contracts.CommandAdmitted)
	return e
}

func phoneOwnerSession(ctx context.Context, g *phonefixture.Gateway) (*webClient, error) {
	var runtime struct {
		APIAddress string `json:"api_address"`
	}
	raw, err := os.ReadFile(filepath.Join(g.Home, "runtime.json"))
	if err != nil {
		return nil, err
	}
	if err = json.Unmarshal(raw, &runtime); err != nil {
		return nil, err
	}
	client := &webClient{home: g.Home, ctx: ctx, origin: "http://" + strings.Replace(runtime.APIAddress, "127.0.0.1", "localhost", 1)}
	output, err := g.Command("web", "enroll", "--host", client.origin)
	if err != nil {
		return nil, err
	}
	link, err := url.Parse(strings.Fields(output)[0])
	if err != nil {
		return nil, err
	}
	authenticator, err := newWebAuthenticator()
	if err != nil {
		return nil, err
	}
	status, _, err := client.signIn(authenticator, "enroll", link.Fragment, "trusted")
	if err != nil {
		return nil, err
	}
	if status != 200 {
		return nil, fmt.Errorf("owner enrollment HTTP %d", status)
	}
	return client, nil
}
