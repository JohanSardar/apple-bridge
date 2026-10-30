package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"regexp"
	"strings"
	"time"
)

func runMacExtrasReal(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC()}
	add := func(n string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: n, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	ctx = world.WithOptions(ctx, world.Options{Scope: "mac:read,mac:write,system:read"})
	w, err := world.NewRealWithSetup(ctx, level, 1, world.NativeSetup{IndependentTruth: true})
	add("world_started", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "mac:read,mac:write,system:read")
	add("scope_granted", err == nil)
	if err != nil {
		return
	}
	client := scripted.Client{Endpoint: endpoint}
	err = w.Await(20*time.Second, func() bool {
		var out contracts.QueryResult
		return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
	})
	add("helper_ready", err == nil)
	if err != nil {
		return
	}
	if s.ID == "IWORK-REAL" {
		installed := 0
		for _, app := range []string{"Pages", "Numbers", "Keynote"} {
			if _, statErr := os.Stat("/Applications/" + app + ".app"); statErr == nil {
				installed++
			}
		}
		e.Metrics = map[string]any{"installed_iwork_app_count": installed}
		var out contracts.CommandRecord
		err = client.Call(ctx, "mac.iwork_export", contracts.Command{ID: contracts.NewCommandID(), Args: json.RawMessage(`{"app":"pages","format":"text","document_blob":"W2FiLXRlc3Rd"}`), WaitMS: 15000}, &out)
		add("iwork_export_observed", err == nil && out.State == contracts.CommandObserved && contracts.MacCompletionEvidence(out.Evidence, out.RequestHash))
		var evidence struct {
			Readback struct {
				Data string `json:"data_base64"`
			} `json:"readback"`
		}
		decode := json.Unmarshal(out.Evidence, &evidence)
		data, blobErr := base64.StdEncoding.DecodeString(evidence.Readback.Data)
		add("exported_tagged_content", decode == nil && blobErr == nil && strings.TrimSpace(string(data)) == "[ab-test]")
		if err != nil || out.State != contracts.CommandObserved {
			e.Status = "limited"
			e.Conditions = map[string]string{"reason": "Native iWork export is unsupported"}
			if installed == 0 {
				e.Conditions["reason"] = "Native iWork export is unsupported; no iWork applications installed on qualification host"
			}
		}
		return
	}
	op := map[string]string{"TM-STATUS": "mac.time_machine_status", "BT-REAL": "mac.bluetooth_devices", "AIRPLAY-REAL": "mac.airplay_routes", "SHAZAM-REAL": "mac.recognize_song"}[s.ID]
	args := json.RawMessage(`{}`)
	if s.ID == "SHAZAM-REAL" {
		args = json.RawMessage(`{"audio_blob":"W2FiLXRlc3Rd"}`)
	}
	var out contracts.QueryResult
	err = client.Call(ctx, op, contracts.Query{Args: args, DeadlineMS: 20000}, &out)
	var result map[string]any
	decode := json.Unmarshal(out.ResultJSON, &result)
	add("native_query_completed", err == nil && decode == nil)
	add("native_operation_available", err == nil && decode == nil && result["limited"] != true)
	if result["limited"] == true {
		e.Status = "limited"
		reasons := map[string]string{"SHAZAM-REAL": "ShazamKit entitlement and fixture qualification required", "BT-REAL": "Native Bluetooth discovery is not implemented", "AIRPLAY-REAL": "Native AirPlay route discovery is not implemented"}
		e.Conditions = map[string]string{"reason": reasons[s.ID]}
		return
	}
	switch s.ID {
	case "TM-STATUS":
		truth, truthErr := exec.CommandContext(ctx, "/usr/bin/tmutil", "status").Output()
		status, _ := result["status"].(string)
		running := regexp.MustCompile(`Running\s*=\s*([01])`)
		observed := running.FindStringSubmatch(status)
		expected := running.FindStringSubmatch(string(truth))
		add("tmutil_status_truth", truthErr == nil && len(observed) == 2 && len(expected) == 2 && observed[1] == expected[1])
	case "BT-REAL":
		_, ok := result["devices"].([]any)
		add("devices_returned", ok)
	case "AIRPLAY-REAL":
		routes, ok := result["routes"].([]any)
		add("routes_returned", ok)
		e.Metrics = map[string]any{"route_count": len(routes)}
		add("airplay_route_available", ok && len(routes) > 0)
		if !ok || len(routes) == 0 {
			e.Status = "limited"
			e.Conditions = map[string]string{"reason": "No AirPlay output route is available on the qualification host; selection remains unqualified"}
			return
		}
		original, originalOK := result["current_device"].(string)
		route, routeOK := routes[0].(map[string]any)
		device, deviceOK := route["device"].(string)
		add("restorable_route_identified", originalOK && original != "" && routeOK && deviceOK && device != "")
		if !originalOK || original == "" || !routeOK || !deviceOK || device == "" {
			return
		}
		command := func(callCtx context.Context, op, device string) bool {
			args, _ := json.Marshal(map[string]string{"device": device})
			var record contracts.CommandRecord
			err := client.Call(callCtx, op, contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &record)
			return err == nil && record.State == contracts.CommandObserved && contracts.MacCompletionEvidence(record.Evidence, record.RequestHash)
		}
		current := func(callCtx context.Context) (string, bool) {
			var response contracts.QueryResult
			callErr := client.Call(callCtx, "mac.airplay_routes", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 20000}, &response)
			var state struct {
				Device string `json:"current_device"`
			}
			decodeErr := json.Unmarshal(response.ResultJSON, &state)
			return state.Device, callErr == nil && decodeErr == nil
		}
		defer func() {
			cleanup, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			add("output_device_restored", command(cleanup, "mac.set_output_device", original))
			actual, readOK := current(cleanup)
			add("output_restore_truth", readOK && actual == original)
		}()
		add("airplay_selection_observed", command(ctx, "mac.airplay_select", device))
		actual, readOK := current(ctx)
		add("airplay_selection_truth", readOK && actual == device)
	case "SHAZAM-REAL":
		title, ok := result["title"].(string)
		add("recognized_fixture", ok && title != "")
	}
	return
}
