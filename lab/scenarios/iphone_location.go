package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
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
	cmd := iphoneProjectCommand(ctx, "xcrun", "swiftc", "-swift-version", "6", "-parse-as-library", "ios/Capabilities/Location/LocationStore.swift", "ios/App/QualificationWait.swift", "lab/fixtures/location/LocationProof.swift", "-o", binary)
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
	gateway, phone, token, err := iphoneBinaryFixture(ctx, dir)
	add("binary_gateway_fixture", err == nil)
	if err != nil {
		return e
	}
	defer gateway.Close()
	readToken, err := gateway.Command("token", "create", "--scope", "iphone:read")
	add("read_grant", err == nil)
	raw, err := os.ReadFile(fixture)
	add("fixture", err == nil)
	for i := 0; i < 2; i++ {
		response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/location-records", raw)
		ok := err == nil && response.StatusCode == 200
		if response != nil {
			response.Body.Close()
		}
		add("signed_records_accepted", ok)
	}
	var result contracts.ReadResult
	err = gateway.Invoke("records.query", contracts.ReadQuery{Collection: "iphone.location", Limit: 100}, readToken, &result)
	add("durable_deduplicated_location_records", err == nil && len(result.Records) == 5)
	invalid, _ := json.Marshal(map[string]any{"device_id": phone.ID, "name": "[ab-test] fence", "latitude": 91, "longitude": 34, "radius": 150})
	status, err := gateway.Request("/v1/op/iphone.geofence.add", contracts.Command{ID: "cmd_loc_invalid", Args: invalid}, token, nil)
	add("invalid_geofence_rejected", err == nil && status == 400)
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
		var admitted contracts.CommandRecord
		err := gateway.Invoke(op, command, token, &admitted)
		add("location_command_admitted", err == nil && admitted.State == contracts.CommandAdmitted)
		if op == "iphone.geofence.add" {
			add("geofence_command_registered", err == nil && admitted.State == contracts.CommandAdmitted)
		}
		response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/pending", nil)
		if err != nil {
			add("pending_command", false)
			return e
		}
		var packets []simapple.IPhonePacket
		err = json.NewDecoder(response.Body).Decode(&packets)
		response.Body.Close()
		add("encrypted_location_command", err == nil && len(packets) == 1)
		if len(packets) != 1 {
			return e
		}
		plain, err := phone.Open(packets[0])
		var job simapple.IPhoneJob
		add("location_command_decrypted", err == nil && json.Unmarshal(plain, &job) == nil && job.Command.Op == op)
		summary := simapple.IPhoneResult{ID: command.ID, Hash: job.Hash, State: contracts.CommandObserved, Data: json.RawMessage(`{"count":1,"significant":true}`)}
		unsafe := summary
		unsafe.Data = json.RawMessage(`{"count":1,"significant":true,"latitude":12,"longitude":34}`)
		unsafeBody, _ := json.Marshal(unsafe)
		unsafeResponse, unsafeErr := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/result", unsafeBody)
		add("coordinate_bearing_receipt_rejected", unsafeErr == nil && unsafeResponse.StatusCode == 400)
		if unsafeResponse != nil {
			unsafeResponse.Body.Close()
		}
		body, _ := json.Marshal(summary)
		response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/result", body)
		add("location_completion_accepted", err == nil && response.StatusCode == 200)
		if response != nil {
			response.Body.Close()
		}
		var completed contracts.CommandRecord
		err = gateway.Invoke("commands.status", map[string]string{"command_id": command.ID}, token, &completed)
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
