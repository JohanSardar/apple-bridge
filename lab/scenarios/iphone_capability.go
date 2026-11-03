package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/phonefixture"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runIPhoneCapability(ctx context.Context, s Scenario, level string, sources ...string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC()}
	if level == "real" {
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "permission", Detail: "limited: real-device permission request and observation harness missing; needs Allow tap"}}
		return e
	}
	dir, err := os.MkdirTemp("", "ab-test-ios-proof-")
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "proof")
	args := append([]string{"swiftc", "-parse-as-library", "-o", binary}, sources...)
	cmd := iphoneProjectCommand(ctx, "xcrun", args...)
	_, err = cmd.CombinedOutput()
	e.Assertions = append(e.Assertions, Assertion{Name: "swift_adapter_build", Passed: err == nil})
	if err != nil {
		return e
	}
	payloadFile := filepath.Join(dir, "result.json")
	output, err := exec.CommandContext(ctx, binary, payloadFile).Output()
	e.Passed = err == nil && (string(output) == "PASS 8\n" || s.ID == "APP-DEVICE-SIM" && string(output) == "PASS 5\n")
	e.Assertions = append(e.Assertions, Assertion{Name: "fake_adapter_functional", Passed: e.Passed})
	if e.Passed {
		ops := []string{"iphone.motion.history", "iphone.motion.activity"}
		if s.ID == "APP-DEVICE-SIM" {
			ops = []string{"iphone.device.status"}
		}
		payload, readErr := os.ReadFile(payloadFile)
		ok := readErr == nil && len(payload) <= 20000 && proveIPhoneCapabilityDelivery(ctx, dir, payload, ops)
		e.Assertions = append(e.Assertions, Assertion{Name: "signed_result_upload_then_next_command", Passed: ok})
		e.Passed = e.Passed && ok
	}
	return e
}

func proveIPhoneCapabilityDelivery(ctx context.Context, dir string, payload []byte, ops []string, admission ...func(bool)) bool {
	fail := func(stage string, err error) bool {
		fmt.Fprintf(os.Stderr, "phone capability fixture %s: %v\n", stage, err)
		return false
	}
	selection := world.ImplementationSelection(ctx)
	goBinary, err := filepath.Abs("bin/bridge")
	if err != nil {
		return fail("Go binary path", err)
	}
	binary, err := selection.Binary(selection.Gateway, goBinary)
	if err != nil {
		return fail("selected binary", err)
	}
	gateway, err := phonefixture.Start(ctx, binary, filepath.Join(dir, "home"))
	if err != nil {
		return fail("gateway startup", err)
	}
	defer gateway.Close()
	output, err := gateway.Command("pair", "--iphone", "--via-remote")
	if err != nil {
		return fail("pairing code", err)
	}
	code := strings.Split(output, "\n")[0]
	phone, err := simapple.NewIPhone()
	if err != nil {
		return false
	}
	status, err := phone.Pair(gateway.Client, gateway.URL, code)
	if err != nil || status != 200 {
		return fail(fmt.Sprintf("pair HTTP %d", status), err)
	}
	token, err := gateway.Command("token", "create", "--scope", "iphone:write")
	if err != nil {
		return fail("API token", err)
	}
	for index, op := range ops {
		args := map[string]any{"device_id": phone.ID}
		if op == "iphone.health.log" {
			args["kind"] = "water"
			args["value"] = 250
			args["start"] = "2026-10-29T12:00:00Z"
			args["tag"] = "[ab-test]"
		}
		if op == "iphone.motion.history" {
			args["days"] = 7
		}
		raw, _ := json.Marshal(args)
		id := []string{"cmd_capability_first", "cmd_capability_next"}[index]
		var admitted contracts.CommandRecord
		err = gateway.Invoke(op, contracts.Command{ID: id, Op: op, Args: raw}, token, &admitted)
		for _, report := range admission {
			report(err == nil && admitted.State == contracts.CommandAdmitted)
		}
		if err != nil || admitted.State != contracts.CommandAdmitted {
			return fail(fmt.Sprintf("%s admission %s", op, admitted.State), err)
		}
		response, err := phone.Request(gateway.Client, gateway.URL, "/v1/iphone/pending", nil)
		if err != nil {
			return false
		}
		packetsRaw, err := io.ReadAll(response.Body)
		response.Body.Close()
		var packets []simapple.IPhonePacket
		if err != nil || json.Unmarshal(packetsRaw, &packets) != nil || len(packets) != 1 {
			return fail(fmt.Sprintf("pending HTTP %d packets %d", response.StatusCode, len(packets)), err)
		}
		plain, err := phone.Open(packets[0])
		if err != nil {
			return false
		}
		var job simapple.IPhoneJob
		if json.Unmarshal(plain, &job) != nil || job.Command.ID != id {
			return false
		}
		resultData := json.RawMessage(payload)
		if index == 1 {
			resultData = json.RawMessage(`{"activity":"walking","confidence":"high"}`)
		}
		result, _ := json.Marshal(simapple.IPhoneResult{ID: id, Hash: job.Hash, State: contracts.CommandObserved, Data: resultData})
		response, err = phone.Request(gateway.Client, gateway.URL, "/v1/iphone/result", result)
		if err != nil {
			return false
		}
		response.Body.Close()
		if response.StatusCode != 200 {
			return false
		}
		var observed contracts.CommandRecord
		err = gateway.Invoke("commands.status", map[string]string{"command_id": id}, token, &observed)
		if err != nil || observed.State != contracts.CommandObserved {
			return fail(fmt.Sprintf("ledger %s", observed.State), err)
		}
	}
	return true
}
