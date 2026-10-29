package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

func runIPhoneLocation(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	if level == "real" {
		e.Passed = false
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "location_permission", Status: "limited", Detail: "needs Allow tap"}}
		return e
	}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-location-")
	add("isolated_home", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "location-proof")
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-swift-version", "6", "-parse-as-library", "ios/Capabilities/Location/LocationStore.swift", "lab/fixtures/location/LocationProof.swift", "-o", binary)
	cmd.Env = append(os.Environ(), "SDKROOT=")
	output, err := cmd.CombinedOutput()
	add("swift_adapter_build", err == nil)
	if err != nil {
		_ = output
		return e
	}
	fixture := filepath.Join(dir, "records.json")
	output, err = exec.CommandContext(ctx, binary, fixture).CombinedOutput()
	add("fake_adapter_current_geofence_events_significant_restart_permission", err == nil && string(output) == "location_adapter_proof=pass\n")
	if err != nil {
		return e
	}
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
	delivery := iphone.NewDelivery(store, a)
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
	raw, err := os.ReadFile(fixture)
	add("fixture", err == nil)
	for i := 0; i < 2; i++ {
		response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/location-records", raw)
		ok := err == nil && response.StatusCode == 200
		if response != nil {
			response.Body.Close()
		}
		add("signed_records_accepted", ok)
	}
	result, err := store.Read(ctx, trust.Visibility{contracts.IPhone: true}, contracts.ReadQuery{Collection: "iphone.location", Limit: 100})
	add("durable_deduplicated_location_records", err == nil && len(result.Records) == 5)
	args, _ := json.Marshal(map[string]any{"device_id": phone.ID, "name": "[ab-test] fence", "latitude": 12, "longitude": 34, "radius": 150})
	add("geofence_command_registered", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_loc", Op: "iphone.geofence.add", Args: args}) == nil)
	invalid, _ := json.Marshal(map[string]any{"device_id": phone.ID, "name": "[ab-test] fence", "latitude": 91, "longitude": 34, "radius": 150})
	add("invalid_geofence_rejected", contracts.ValidateIPhoneCommand(contracts.Command{ID: "cmd_loc", Op: "iphone.geofence.add", Args: invalid}) != nil)

	add("command_ledger", store.InitCommands(ctx) == nil)
	gateway := &operations.LocalGateway{State: store, Authority: a, Context: ctx, IPhone: delivery}
	delivery.Authorize = gateway.ReauthorizeIPhone
	token, err := a.CreateToken([]contracts.Grant{"iphone:write"})
	add("command_grant", err == nil)
	principal, err := a.AuthenticateToken(token)
	add("command_principal", err == nil)
	for index, op := range []string{"iphone.location.current", "iphone.geofence.add", "iphone.geofence.list", "iphone.geofence.remove", "iphone.location.significant"} {
		values := map[string]any{"device_id": phone.ID}
		switch op {
		case "iphone.geofence.add":
			values["name"] = "[ab-test] fence"
			values["latitude"] = 12
			values["longitude"] = 34
			values["radius"] = 150
		case "iphone.geofence.remove":
			values["name"] = "[ab-test] fence"
		case "iphone.location.significant":
			values["enabled"] = true
		}
		input, _ := json.Marshal(values)
		command := contracts.Command{ID: fmt.Sprintf("cmd_location_%d", index), Op: op, Args: input}
		admitted, err := gateway.Submit(ctx, principal, command)
		add("location_command_admitted", err == nil && admitted.State == contracts.CommandAdmitted)
		response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/pending", nil)
		if err != nil {
			add("pending_command", false)
			return e
		}
		var packets []iphone.Packet
		err = json.NewDecoder(response.Body).Decode(&packets)
		response.Body.Close()
		add("encrypted_location_command", err == nil && len(packets) == 1)
		if len(packets) != 1 {
			return e
		}
		plain, err := iphone.Open(phone.Encryption, phone.ID, packets[0], phone.Gateway)
		var job iphone.Job
		add("location_command_decrypted", err == nil && json.Unmarshal(plain, &job) == nil && job.Command.Op == op)
		summary := iphone.Result{ID: command.ID, Hash: job.Hash, State: contracts.CommandObserved, Data: json.RawMessage(`{"count":1,"significant":true}`)}
		unsafe := summary
		unsafe.Data = json.RawMessage(`{"count":1,"significant":true,"latitude":12,"longitude":34}`)
		unsafeBody, _ := json.Marshal(unsafe)
		unsafeResponse, unsafeErr := phone.Request(server.Client(), server.URL, "/v1/iphone/result", unsafeBody)
		add("coordinate_bearing_receipt_rejected", unsafeErr == nil && unsafeResponse.StatusCode == 400)
		if unsafeResponse != nil {
			unsafeResponse.Body.Close()
		}
		body, _ := json.Marshal(summary)
		response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/result", body)
		add("location_completion_accepted", err == nil && response.StatusCode == 200)
		if response != nil {
			response.Body.Close()
		}
		completed, err := store.Command(ctx, command.ID)
		add("location_completion_observed", err == nil && completed.State == contracts.CommandObserved)
		var receipt struct {
			Payload []byte `json:"payload"`
		}
		var payload struct {
			Data map[string]any `json:"data"`
		}
		ok := json.Unmarshal(completed.Evidence, &receipt) == nil && json.Unmarshal(receipt.Payload, &payload) == nil && len(payload.Data) == 2 && payload.Data["count"] == float64(1) && payload.Data["significant"] == true
		add("coordinate_free_receipt", ok)
	}
	return e
}
