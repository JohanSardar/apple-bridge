package scenarios

import (
	"context"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/blobs"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func runIPhoneCapture(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "camera_microphone_permission", Status: "limited", Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
		return e
	}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-capture-")
	add("isolated_home", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-parse-as-library", "ios/Capabilities/Capture/CaptureModel.swift", "ios/App/QualificationWait.swift", "lab/ios/CaptureProof.swift", "-o", filepath.Join(dir, "proof"))
	cmd.Env = append(os.Environ(), "SDKROOT=")
	_, err = cmd.CombinedOutput()
	add("swift_capture_build", err == nil)
	if err != nil {
		return e
	}
	output, err := exec.CommandContext(ctx, filepath.Join(dir, "proof")).Output()
	add("face_id_denial_and_memory_upload_paths", err == nil && string(output) == "PASS 4\n")
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
	defer store.Close()
	add("ledger", store.InitCommands(ctx) == nil)
	bs, err := blobs.Open(dir, blobs.Config{})
	add("blob_store", err == nil)
	if err != nil {
		return e
	}
	delivery := iphone.NewDelivery(store, a)
	delivery.Blobs = bs
	gateway := &operations.LocalGateway{State: store, Authority: a, Context: ctx, IPhone: delivery}
	delivery.Authorize = gateway.ReauthorizeIPhone
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
	add("token", err == nil)
	principal, err := a.AuthenticateToken(token)
	add("principal", err == nil)
	request := func(path string, payload any) (int, []byte) {
		raw, _ := json.Marshal(payload)
		response, err := phone.Request(server.Client(), server.URL, path, raw)
		if err != nil {
			return 0, nil
		}
		defer response.Body.Close()
		data, _ := io.ReadAll(response.Body)
		return response.StatusCode, data
	}
	for _, seconds := range []int{0, 11} {
		args, _ := json.Marshal(map[string]any{"device_id": phone.ID, "seconds": seconds})
		add("audio_duration_bound_rejected", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_capture_bounds", Op: "iphone.capture.audio", Args: args}) != nil)
	}
	for _, kind := range []string{"photo", "audio"} {
		args, _ := json.Marshal(contracts.IPhoneTarget{DeviceID: phone.ID})
		command := contracts.Command{ID: "cmd_capture_" + kind, Op: "iphone.capture." + kind, Args: args}
		r, err := gateway.Submit(ctx, principal, command)
		add(kind+"_admitted", err == nil && r.State == contracts.CommandAdmitted)
		contentType := "image/jpeg"
		if kind == "audio" {
			contentType = "audio/wav"
		}
		upload := iphone.CaptureUpload{ID: command.ID, Hash: r.RequestHash, ContentType: contentType, Data: []byte("[ab-test] " + kind)}
		status, _ := request("/v1/iphone/capture", upload)
		add(kind+"_undispatched_upload_rejected", status == 400)
		status, _ = request("/v1/iphone/pending", nil)
		add(kind+"_dispatched", status == 200)
		upload.Hash = "wrong"
		status, _ = request("/v1/iphone/capture", upload)
		add(kind+"_wrong_hash_rejected", status == 400)
		upload.Hash = r.RequestHash
		status, raw := request("/v1/iphone/capture", upload)
		var receipt iphone.CaptureReceipt
		add(kind+"_signed_blob_upload", status == 200 && json.Unmarshal(raw, &receipt) == nil && receipt.Size == int64(len(upload.Data)))
		grant, err := store.QueryBlobGrant(ctx, principal, receipt.BlobID)
		add(kind+"_caller_blob_grant", err == nil)
		reader, err := bs.OpenAuthorized(ctx, grant, receipt.Digest)
		var data []byte
		if err == nil {
			data, err = io.ReadAll(reader)
			reader.Close()
		}
		add(kind+"_gateway_bytes_match", err == nil && string(data) == string(upload.Data))
		status, _ = request("/v1/iphone/capture", upload)
		add(kind+"_upload_retry_idempotent", status == 200)
		upload.Data = []byte("different")
		status, _ = request("/v1/iphone/capture", upload)
		add(kind+"_changed_retry_rejected", status == 400)
		result, _ := json.Marshal(receipt)
		status, _ = request("/v1/iphone/result", iphone.Result{ID: command.ID, Hash: r.RequestHash, State: contracts.CommandObserved, Data: result})
		add(kind+"_signed_completion", status == 200)
		observed, err := store.Command(ctx, command.ID)
		add(kind+"_ledger_observed", err == nil && observed.State == contracts.CommandObserved)
	}
	delegated := trust.Principal{ID: "replica:test/agent:capture", Kind: "delegated", Level: trust.AgentFull, Grants: []contracts.Grant{"iphone:write"}}
	_, err = a.Principal(delegated.ID)
	add("delegated_principal_has_no_local_credential", err != nil)
	args, _ := json.Marshal(contracts.IPhoneTarget{DeviceID: phone.ID})
	command := contracts.Command{ID: "cmd_capture_delegated", Op: "iphone.capture.photo", Args: args}
	hash, err := contracts.CommandHash(command)
	add("delegated_command_hash", err == nil)
	era, err := store.AdmissionEra(ctx)
	add("delegated_admission_era", err == nil)
	record := contracts.CommandRecord{AdmissionEra: era, Command: command, Principal: delegated.ID, Source: contracts.IPhone, RequestHash: hash, State: contracts.CommandAdmitted, CreatedAt: time.Now(), ExpiresAt: time.Now().Add(time.Minute)}
	record, err = store.TransitionCommand(ctx, state.CommandTransition{Record: record})
	add("delegated_admitted_fixture", err == nil)
	for _, next := range []contracts.CommandState{contracts.CommandPrepared, contracts.CommandDispatched} {
		previous := record.State
		record.State = next
		record, err = store.TransitionCommand(ctx, state.CommandTransition{From: previous, Record: record})
		add("delegated_"+string(next)+"_fixture", err == nil)
	}
	authorized := false
	delivery.Authorize = func(_ context.Context, r contracts.CommandRecord) error {
		authorized = r.Principal == delegated.ID && r.RequestHash == hash
		if !authorized {
			return contracts.ErrUnauthorized
		}
		return nil
	}
	upload := iphone.CaptureUpload{ID: command.ID, Hash: hash, ContentType: "image/jpeg", Data: []byte("[ab-test] delegated")}
	status, raw := request("/v1/iphone/capture", upload)
	var receipt iphone.CaptureReceipt
	add("delegated_reauthorized_upload", authorized && status == 200 && json.Unmarshal(raw, &receipt) == nil)
	grant, err := store.QueryBlobGrant(ctx, delegated, receipt.BlobID)
	add("delegated_caller_blob_grant", err == nil && grant.AllowsBlob(receipt.Digest))
	delivery.Authorize = func(context.Context, contracts.CommandRecord) error { return contracts.ErrUnauthorized }
	status, _ = request("/v1/iphone/capture", upload)
	add("delegated_revoked_upload_rejected", status == 400)
	return e
}
