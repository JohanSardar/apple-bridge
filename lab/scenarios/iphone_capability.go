package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func runIPhoneCapability(ctx context.Context, s Scenario, level string, sources ...string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC()}
	if level == "real" {
		e.Status = "limited"
		e.Assertions = []Assertion{{Name: "permission", Detail: "limited: needs Allow tap"}}
		return e
	}
	dir, err := os.MkdirTemp("", "ab-test-ios-proof-")
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "proof")
	args := append([]string{"swiftc", "-parse-as-library", "-o", binary}, sources...)
	cmd := exec.CommandContext(ctx, "xcrun", args...)
	cmd.Env = append(os.Environ(), "SDKROOT=")
	_, err = cmd.CombinedOutput()
	e.Assertions = append(e.Assertions, Assertion{Name: "swift_adapter_build", Passed: err == nil})
	if err != nil {
		return e
	}
	payloadFile := filepath.Join(dir, "result.json")
	output, err := exec.CommandContext(ctx, binary, payloadFile).Output()
	e.Passed = err == nil && string(output) == "PASS 8\n"
	e.Assertions = append(e.Assertions, Assertion{Name: "fake_adapter_functional", Passed: e.Passed})
	if e.Passed {
		ops := []string{"iphone.motion.history", "iphone.motion.activity"}
		if s.ID == "APP-DEVICE-SIM" {
			ops = []string{"iphone.device.status", "iphone.nfc.read"}
		}
		payload, readErr := os.ReadFile(payloadFile)
		ok := readErr == nil && len(payload) <= 20000 && proveIPhoneCapabilityDelivery(ctx, dir, payload, ops)
		e.Assertions = append(e.Assertions, Assertion{Name: "signed_result_upload_then_next_command", Passed: ok})
		e.Passed = e.Passed && ok
	}
	return e
}

func proveIPhoneCapabilityDelivery(ctx context.Context, dir string, payload []byte, ops []string) bool {
	authority, err := trust.Open(filepath.Join(dir, "trust"))
	if err != nil {
		return false
	}
	store, err := state.Open(filepath.Join(dir, "state"))
	if err != nil {
		return false
	}
	defer store.Close()
	if store.InitCommands(ctx) != nil {
		return false
	}
	delivery := iphone.NewDelivery(store, authority)
	gateway := &operations.LocalGateway{State: store, Authority: authority, Context: ctx, IPhone: delivery}
	delivery.Authorize = gateway.ReauthorizeIPhone
	mux := http.NewServeMux()
	mux.Handle("/v1/iphone/pair", authority.IPhoneHandler())
	mux.Handle("/v1/iphone/", delivery.Handler())
	server := httptest.NewTLSServer(mux)
	defer server.Close()
	code, err := authority.PairIPhoneCode(server.URL, false, time.Minute)
	if err != nil {
		return false
	}
	phone, err := simapple.NewIPhone()
	if err != nil {
		return false
	}
	status, err := phone.Pair(server.Client(), server.URL, code)
	if err != nil || status != 200 {
		return false
	}
	token, err := authority.CreateToken([]contracts.Grant{"iphone:write"})
	if err != nil {
		return false
	}
	principal, err := authority.AuthenticateToken(token)
	if err != nil {
		return false
	}
	for index, op := range ops {
		args := map[string]any{"device_id": phone.ID}
		if op == "iphone.motion.history" {
			args["days"] = 7
		}
		raw, _ := json.Marshal(args)
		id := []string{"cmd_capability_first", "cmd_capability_next"}[index]
		admitted, err := gateway.Submit(ctx, principal, contracts.Command{ID: id, Op: op, Args: raw})
		if err != nil || admitted.State != contracts.CommandAdmitted {
			return false
		}
		response, err := phone.Request(server.Client(), server.URL, "/v1/iphone/pending", nil)
		if err != nil {
			return false
		}
		packetsRaw, err := io.ReadAll(response.Body)
		response.Body.Close()
		var packets []iphone.Packet
		if err != nil || json.Unmarshal(packetsRaw, &packets) != nil || len(packets) != 1 {
			return false
		}
		plain, err := iphone.Open(phone.Encryption, phone.ID, packets[0], phone.Gateway)
		if err != nil {
			return false
		}
		var job iphone.Job
		if json.Unmarshal(plain, &job) != nil || job.Command.ID != id {
			return false
		}
		resultData := json.RawMessage(payload)
		if index == 1 {
			resultData = json.RawMessage(`{"activity":"walking","confidence":"high"}`)
			if op == "iphone.nfc.read" {
				resultData = json.RawMessage(`[{"format":1,"type":"VA==","identifier":"","payload":"AmVuYQ=="}]`)
			}
		}
		result, _ := json.Marshal(iphone.Result{ID: id, Hash: job.Hash, State: contracts.CommandObserved, Data: resultData})
		response, err = phone.Request(server.Client(), server.URL, "/v1/iphone/result", result)
		if err != nil {
			return false
		}
		response.Body.Close()
		if response.StatusCode != 200 {
			return false
		}
		observed, err := store.Command(ctx, id)
		if err != nil || observed.State != contracts.CommandObserved {
			return false
		}
	}
	return true
}
