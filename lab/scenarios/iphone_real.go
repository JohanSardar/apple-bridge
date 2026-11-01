package scenarios

import (
	"bufio"
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"sync/atomic"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/iphone"
	"github.com/JohanSardar/apple-bridge/internal/operations"
	"github.com/JohanSardar/apple-bridge/internal/relay"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type phoneReceipt struct {
	Run     string            `json:"run"`
	Done    bool              `json:"done"`
	Checks  map[string]bool   `json:"checks"`
	Counts  map[string]int    `json:"counts"`
	Digests map[string]string `json:"digests"`
}

func runIPhoneReal(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	check := func(name string, pass bool) bool {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: pass})
		e.Passed = e.Passed && pass
		return pass
	}
	fail := func(name string, err error) Evidence {
		check(name, false)
		if err != nil {
			sum := sha256.Sum256([]byte(err.Error()))
			e.Metrics["error_digest"] = hex.EncodeToString(sum[:])
		}
		return e
	}
	device := os.Getenv("AB_REAL_DEVICE")
	if device == "" {
		return fail("physical_device_selected", nil)
	}
	if os.Getenv("APPLE_BRIDGE_SECRET_STORE") != "file" {
		return fail("isolated_file_secret_store", nil)
	}
	if s.ID == "FOC-PHONE-SYNC" {
		logPhoneInstruction("waiting for the Air Focus lock before starting phone observation")
		release, err := lockAirFocus(ctx)
		if err != nil {
			return fail("air_focus_lock", err)
		}
		defer release()
		check("air_focus_lock", true)
	}
	if err := exec.CommandContext(ctx, "python3", "lab/iphone-real/device.py", device).Run(); err != nil {
		return fail("physical_device_available", err)
	}
	lock := exec.CommandContext(ctx, "lockf", "-k", "-t", "0", "/tmp/ab-iphone.lock", "sh", "-c", "printf 'locked\\n'; cat >/dev/null")
	input, err := lock.StdinPipe()
	if err != nil {
		return fail("phone_lock", err)
	}
	output, err := lock.StdoutPipe()
	if err != nil {
		return fail("phone_lock", err)
	}
	if err = lock.Start(); err != nil {
		return fail("phone_lock", err)
	}
	defer func() { input.Close(); lock.Wait() }()
	line, err := bufio.NewReader(output).ReadString('\n')
	if err != nil || line != "locked\n" {
		return fail("phone_lock", err)
	}
	dir, err := os.MkdirTemp("", "ab-test-real-phone-")
	if err != nil {
		return fail("temporary_home", err)
	}
	defer os.RemoveAll(dir)
	random := make([]byte, 12)
	if _, err = rand.Read(random); err != nil {
		return fail("run_nonce", err)
	}
	run := hex.EncodeToString(random)
	commandID := "cmd_ab_test_" + run
	request := map[string]string{"run": run, "scenario": s.ID, "command": commandID}
	var gateway *operations.LocalGateway
	var authority *trust.Authority
	var store *state.Store
	var delivery *iphone.Delivery
	var connector atomic.Pointer[realPhoneConnector]
	var pushes atomic.Int64
	var pushFailures atomic.Int64
	paired := s.ID == "APP-PAIR-REAL" || s.ID == "APP-CMD-REAL" || s.ID == "APP-NOTIFY-REAL" || s.ID == "APP-APPROVE-REAL"
	if paired {
		authority, err = trust.Open(filepath.Join(dir, "trust"))
		if err != nil {
			return fail("authority", err)
		}
		store, err = state.Open(filepath.Join(dir, "state"))
		if err != nil {
			return fail("ledger", err)
		}
		defer store.Close()
		if err = store.InitCommands(ctx); err != nil {
			return fail("ledger", err)
		}
		delivery = iphone.NewDelivery(store, authority)
		gateway = &operations.LocalGateway{State: store, Authority: authority, Context: ctx, IPhone: delivery}
		delivery.Authorize = gateway.ReauthorizeIPhone
		delivery.Decide = gateway.DecidePhoneApproval
		delivery.ConnectorPending = func() []iphone.ConnectorApproval {
			if fixture := connector.Load(); fixture != nil {
				return fixture.pendingPhone()
			}
			return nil
		}
		delivery.ConnectorDecide = func(id, code string, approve bool) error {
			if fixture := connector.Load(); fixture != nil {
				return fixture.approvals.Decide(id, code, approve)
			}
			return contracts.ErrUnauthorized
		}
		mux := realPhoneHandler(authority, delivery)
		server := httptest.NewUnstartedServer(mux)
		listener, listenErr := net.Listen("tcp4", "0.0.0.0:0")
		if listenErr != nil {
			return fail("phone_https", listenErr)
		}
		server.Listener.Close()
		server.Listener = listener
		server.StartTLS()
		defer server.Close()
		authority.Fingerprint = trust.Fingerprint(server.Certificate().Raw)
		address := os.Getenv("AB_REAL_HOST")
		if address == "" {
			raw, ipErr := exec.CommandContext(ctx, "ipconfig", "getifaddr", "en0").Output()
			if ipErr != nil {
				return fail("reachable_host", ipErr)
			}
			address = strings.TrimSpace(string(raw))
		}
		code, codeErr := authority.PairIPhoneCode(fmt.Sprintf("https://%s:%d", address, listener.Addr().(*net.TCPAddr).Port), false, 10*time.Minute)
		if codeErr != nil {
			return fail("pair_code", codeErr)
		}
		request["code"] = code
		if s.ID != "APP-PAIR-REAL" {
			key := os.Getenv("AB_REAL_APNS_KEY")
			if key == "" {
				home, _ := os.UserHomeDir()
				key = filepath.Join(home, ".config/apple-bridge/apns/AuthKey_L2854KRF5Z.p8")
			}
			apns, loadErr := relay.LoadAPNs(key, "L2854KRF5Z", "AX3C2B78YD", true)
			if loadErr != nil {
				return fail("apns_credentials", loadErr)
			}
			delivery.Push = func(ctx context.Context, token string, packet iphone.Packet, expires time.Time) error {
				raw, _ := json.Marshal(packet)
				err := apns.Send(ctx, token, raw, expires)
				if err == nil {
					pushes.Add(1)
				} else {
					pushFailures.Add(1)
				}
				return err
			}
			runctx, cancel := context.WithCancel(ctx)
			defer cancel()
			go delivery.Run(runctx)
		}
	}
	var offline time.Duration
	devctl := func(args ...string) error {
		args = append([]string{"devicectl", "--timeout", "15"}, args...)
		var err error
		for {
			err = exec.CommandContext(ctx, "xcrun", args...).Run()
			if err == nil || ctx.Err() != nil || exec.CommandContext(ctx, "python3", "lab/iphone-real/device.py", device, "--check").Run() == nil {
				break
			}
			start := time.Now()
			reconnectErr := exec.CommandContext(ctx, "python3", "lab/iphone-real/device.py", device).Run()
			offline += time.Since(start)
			if reconnectErr != nil {
				return reconnectErr
			}
		}
		if err == nil && slices.Contains(args, "launch") && os.Getenv("AB_WDA_URL") != "" {
			err = exec.CommandContext(ctx, "python3", "lab/iphone-real/activate.py", "com.johansardar.applebridge.ios").Run()
		}
		return err
	}
	source := filepath.Join(dir, "request.json")
	raw, _ := json.Marshal(request)
	if err = os.WriteFile(source, raw, 0600); err != nil {
		return fail("request_file", err)
	}
	bundle := "com.johansardar.applebridge.ios"
	defer func() {
		restored := false
		if devctl("device", "process", "launch", "--device", device, "--terminate-existing", bundle, "--ab-real-restore", run) == nil {
			for attempt := 0; attempt < 10; attempt++ {
				dest := filepath.Join(dir, "restored.json")
				value, copyErr := copyRealPhoneReceipt(devctl, device, bundle, "Documents/ab-real-acknowledged.json", dest)
				if copyErr == nil && value.Run == run && value.Done && value.Checks["state_restored"] && value.Checks["resources_cleaned"] {
					restored = true
					break
				}
				time.Sleep(time.Second)
			}
		}
		check("restoration_receipt", restored)
	}()
	if err = devctl("device", "copy", "to", "--device", device, "--source", source, "--destination", "Documents/ab-real-request.json", "--domain-type", "appDataContainer", "--domain-identifier", bundle); err != nil {
		return fail("copy_request", err)
	}
	if err = devctl("device", "process", "launch", "--device", device, "--terminate-existing", bundle, "--ab-real"); err != nil {
		return fail("launch_probe", err)
	}
	logPhoneInstruction(phoneInstruction(s.ID))

	deadline := time.Now().Add(800 * time.Second)
	submitted := false
	var receipt phoneReceipt
	phase := 0
	focusSubmitted := false
	phaseStart := time.Now()
	phaseOffline := offline
	for time.Now().Before(deadline.Add(offline)) {
		if time.Since(phaseStart)-(offline-phaseOffline) > 210*time.Second {
			break
		}
		if ctx.Err() != nil {
			return fail("run_context", ctx.Err())
		}
		dest := filepath.Join(dir, "receipt.json")
		current, copyErr := copyRealPhoneReceipt(devctl, device, bundle, "Documents/ab-real-result.json", dest)
		if copyErr == nil {
			if current.Run == run {
				receipt = current
				if current.Counts["phase"] != phase {
					phase = current.Counts["phase"]
					phaseStart = time.Now()
					phaseOffline = offline
					logPhoneInstruction(phonePhaseInstruction(s.ID, phase))
				}
				if time.Since(phaseStart)-(offline-phaseOffline) > 210*time.Second {
					break
				}
				if s.ID == "APP-APPROVE-REAL" && phase == 3 && connector.Load() == nil {
					fixture, fixtureErr := newRealPhoneConnector(authority)
					if fixtureErr != nil {
						return fail("connector_request_created", fixtureErr)
					}
					connector.Store(fixture)
					logPhoneInstruction("open [ab-test] iPhone connection, match Code " + fixture.pending.Code + ", tap Allow, and complete Face ID")
					delivery.Wake()
				}
				if s.ID == "FOC-PHONE-SYNC" && receipt.Checks["ready"] && !focusSubmitted {
					focusSubmitted = true
					focus := runAirFocusReal(ctx)
					e.Assertions = append(e.Assertions, focus.Assertions...)
					check("mac_focus_set_and_restore", focus.Passed)
				}
				if receipt.Checks["ready"] && !submitted && gateway != nil {
					phones, listErr := authority.IPhones()
					if listErr != nil || len(phones) != 1 {
						return fail("paired_device_count", listErr)
					}
					args := map[string]any{"device_id": phones[0].ID}
					op := "iphone.ping"
					if s.ID == "APP-NOTIFY-REAL" {
						op = "iphone.notify"
						args["title"] = "[ab-test] Notification qualification"
						args["body"] = "[ab-test] Confirm receipt"
						args["buttons"] = []map[string]string{{"id": "confirmed", "title": "[ab-test] Confirm"}}
					}
					raw, _ := json.Marshal(args)
					token, tokenErr := authority.CreateToken([]contracts.Grant{"iphone:write"})
					if tokenErr != nil {
						return fail("test_principal", tokenErr)
					}
					principal, authErr := authority.AuthenticateToken(token)
					if authErr != nil {
						return fail("test_principal", authErr)
					}
					if s.ID == "APP-APPROVE-REAL" {
						_, token, err = authority.CreateAgent("[ab-test] Face ID", trust.AgentApprove, []contracts.Grant{"iphone:write"})
						if err != nil {
							return fail("approval_agent", err)
						}
						principal, err = authority.AuthenticateToken(token)
						if err != nil {
							return fail("approval_agent", err)
						}
					}
					_, err = gateway.Submit(ctx, principal, contracts.Command{ID: commandID, Op: op, Args: raw, WaitMS: 0})
					if err != nil {
						return fail("admit_phone_command", err)
					}
					submitted = true
				}
				if receipt.Done {
					break
				}
			}
		}
		select {
		case <-ctx.Done():
			return fail("run_context", ctx.Err())
		case <-time.After(time.Second):
		}
	}
	if s.ID == "FOC-PHONE-SYNC" {
		check("mac_focus_step_submitted", focusSubmitted)
	}
	check("fresh_completed_phone_receipt", receipt.Run == run && receipt.Done)
	check("state_restored", receipt.Checks["state_restored"])
	check("resources_cleaned", receipt.Checks["resources_cleaned"])
	for _, name := range phoneRequirements(s.ID) {
		check(name, receipt.Checks[name])
	}
	for name, pass := range receipt.Checks {
		check(name, pass)
	}
	for name, count := range receipt.Counts {
		e.Metrics[name] = count
	}
	for name, digest := range receipt.Digests {
		e.Metrics[name+"_digest"] = digest
	}
	if paired {
		check("signed_pair_status", receipt.Checks["signed_pair_status"])
		check("identity_restored", receipt.Checks["identity_restored"])
	}
	if s.ID == "APP-CMD-REAL" || s.ID == "APP-NOTIFY-REAL" || s.ID == "APP-APPROVE-REAL" {
		record, recordErr := store.Command(ctx, commandID)
		check("gateway_observed_signed_result", recordErr == nil && record.State == contracts.CommandObserved)
		var signed trust.IPhoneRequest
		var result iphone.Result
		valid := json.Unmarshal(record.Evidence, &signed) == nil && json.Unmarshal(signed.Payload, &result) == nil
		var payload map[string]any
		valid = valid && json.Unmarshal(result.Data, &payload) == nil
		if s.ID == "APP-NOTIFY-REAL" {
			check("notification_action_observed", valid && payload["action_id"] == "confirmed")
		} else {
			check("phone_pong_observed", valid && payload["pong"] == true)
		}
		if s.ID == "APP-APPROVE-REAL" {
			fixture := connector.Load()
			check("connector_oauth_token_authenticated", fixture != nil && fixture.qualified())
			check("face_id_succeeded", receipt.Checks["face_id_succeeded"])
		}
		check("apns_accepted", pushes.Load() > 0)
		e.Metrics["apns_accepted_count"] = pushes.Load()
		e.Metrics["apns_failure_count"] = pushFailures.Load()
	}
	if !e.Passed {
		e.Status = "failed"
	}
	return e
}

func logPhoneInstruction(instruction string) {
	instruction = strings.ReplaceAll(strings.ReplaceAll(instruction, "\n", " "), "\r", " ")
	fmt.Fprintln(os.Stderr, "Automation step: "+instruction)
	path := os.Getenv("AB_IPHONE_AUTOMATION_STEPS")
	if path == "" {
		return
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0600)
	if err != nil {
		fmt.Fprintln(os.Stderr, "Could not append the iPhone automation instructions")
		return
	}
	defer file.Close()
	fmt.Fprintln(file, time.Now().Format("15:04")+" "+instruction)
}

func phonePhaseInstruction(s string, phase int) string {
	if s == "APP-INTENTS-REAL" && phase == 2 {
		return "in Shortcuts touch and hold [ab-test] Ping iPhone, tap Delete and Delete Shortcut, return to Apple Bridge, then tap Confirm observations"
	}
	return phoneInstruction(s)
}

func phoneInstruction(s string) string {
	switch s {
	case "FOC-PHONE-SYNC":
		return "observe the existing phone Focus grant while the Air enables and restores Do Not Disturb; no taps required"
	case "APP-SHELL-REAL":
		return "tap Capability, inspect its fields and tap Back; tap Diagnostics, inspect its fields and tap Back, then tap Confirm observations"
	case "APP-PAIR-REAL":
		return "tap Allow on Apple Bridge Local Network if prompted"
	case "APP-CMD-REAL":
		return "tap Allow on Apple Bridge Local Network if prompted; keep the phone unlocked"
	case "APP-NOTIFY-REAL":
		return "tap Allow on Notifications if prompted, touch and hold the [ab-test] Notification qualification banner, then tap [ab-test] Confirm"
	case "APP-APPROVE-REAL":
		return "tap Allow on Notifications if prompted, open the [ab-test] approval, tap Approve and complete Face ID"
	case "APP-LIVE-REAL":
		return "view [ab-test] Updated on the Lock Screen and the existing Bridge status widget, then return to Apple Bridge and tap Confirm observations"
	default:
		return "in Shortcuts tap +, search Check Bridge Status, tap Apple Bridge Check Bridge Status, name it [ab-test] Ping iPhone, tap Done, then tap the shortcut to run it and return to Apple Bridge"
	}
}

func isIPhoneRealScenario(id, level string) bool {
	return level == "real" && slices.Contains([]string{"APP-SHELL-REAL", "APP-PAIR-REAL", "APP-CMD-REAL", "APP-NOTIFY-REAL", "APP-APPROVE-REAL", "APP-LIVE-REAL", "APP-INTENTS-REAL", "FOC-PHONE-SYNC"}, id)
}

func phoneRequirements(s string) []string {
	switch s {
	case "FOC-PHONE-SYNC":
		return []string{"focus_authorized", "phone_initially_inactive", "phone_focus_enabled", "phone_focus_restored"}
	case "APP-SHELL-REAL":
		return []string{"capability_fields_observed", "diagnostics_observed"}
	case "APP-PAIR-REAL":
		return []string{"signed_pair_status", "identity_restored"}
	case "APP-CMD-REAL":
		return []string{"ready", "signed_result_uploaded", "apns_wake", "identity_restored"}
	case "APP-NOTIFY-REAL":
		return []string{"notification_authorized", "ready", "signed_result_uploaded", "identity_restored"}
	case "APP-APPROVE-REAL":
		return []string{"notification_authorized", "ready", "face_id_succeeded", "signed_result_uploaded", "identity_restored", "connector_prompt_code_and_scopes", "connector_face_id_succeeded", "connector_decision_uploaded"}
	case "APP-LIVE-REAL":
		return []string{"activities_authorized", "activity_started", "activity_updated", "widget_shared_status", "live_and_widget_observed", "activity_ended"}
	case "APP-INTENTS-REAL":
		return []string{"saved_shortcut_invoked", "shortcut_pong", "shortcut_cleaned_up"}
	}
	return []string{"unsupported_scenario"}
}

func realPhoneHandler(authority *trust.Authority, delivery *iphone.Delivery) http.Handler {
	mux := http.NewServeMux()
	mux.Handle("/v1/iphone/pair", authority.IPhoneHandler())
	mux.Handle("/v1/iphone/status", authority.IPhoneHandler())
	mux.Handle("/v1/iphone/", delivery.Handler())
	return mux
}

func copyRealPhoneReceipt(copy func(...string) error, device, bundle, source, destination string) (phoneReceipt, error) {
	var receipt phoneReceipt
	if err := os.Remove(destination); err != nil && !os.IsNotExist(err) {
		return receipt, err
	}
	if err := copy("device", "copy", "from", "--device", device, "--source", source, "--destination", destination, "--domain-type", "appDataContainer", "--domain-identifier", bundle); err != nil {
		return receipt, err
	}
	raw, err := os.ReadFile(destination)
	if err != nil {
		return receipt, err
	}
	err = json.Unmarshal(raw, &receipt)
	return receipt, err
}
