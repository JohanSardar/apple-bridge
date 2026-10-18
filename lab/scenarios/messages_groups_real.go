package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func nativeTestContacts() (map[string]string, error) {
	data, err := os.ReadFile(filepath.Join(os.Getenv("HOME"), ".config", "apple-bridge", "test-contacts.env"))
	result := map[string]string{}
	for _, line := range strings.Split(string(data), "\n") {
		key, value, ok := strings.Cut(strings.TrimPrefix(strings.TrimSpace(line), "export "), "=")
		if ok {
			result[strings.TrimSpace(key)] = strings.Trim(strings.TrimSpace(value), "\"'")
		}
	}
	return result, err
}

func runNativeMessagesGroup(ctx context.Context, s Scenario, e Evidence, w *world.World, own []string) (final Evidence) {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	contacts, err := nativeTestContacts()
	add("approved_contacts_configured", err == nil && contacts["REMY_IMESSAGE_TEST_HANDLE"] != "" && contacts["BEN_GROUP_TEST_HANDLE"] != "")
	if err != nil {
		return e
	}
	client := scripted.Client{Endpoint: w.Gateway}
	submit := func(op string, args map[string]any) (contracts.CommandRecord, error) {
		raw, _ := json.Marshal(args)
		command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages." + op, Args: raw, WaitMS: 15000}
		var result contracts.CommandRecord
		err := client.Call(ctx, command.Op, command, &result)
		return result, err
	}
	remy := contacts["REMY_IMESSAGE_TEST_HANDLE"]
	if remy == "" {
		remy = contacts["REMY_IMESSAGE_TEST_EMAIL"]
	}
	tag := "[ab-test " + e.RunID + "]"
	defer func() {
		ok, metrics := cleanupNativeMessageRun(w, e.RunID, true)
		final.Assertions = append(final.Assertions, Assertion{Name: "tagged_messages_cleanup", Passed: ok})
		if final.Metrics == nil {
			final.Metrics = map[string]any{}
		}
		final.Metrics["cleanup"] = metrics
		final.Passed = len(final.Assertions) > 0 && final.Status == ""
		for _, assertion := range final.Assertions {
			final.Passed = final.Passed && assertion.Passed
		}
	}()

	intro, problem := submit("send", map[string]any{"handles": []string{remy}, "service": "iMessage", "text": tag + " This is an automated test of Johan's Apple Bridge and can be ignored."})
	e.Metrics = map[string]any{"intro_state": intro.State, "intro_disposition": intro.Disposition}
	if problem != nil {
		e.Metrics["intro_error"] = problem.Error()
	}
	add("remy_automated_test_notice_observed", problem == nil && (intro.State == contracts.CommandObserved || intro.State == contracts.CommandDelivered))
	if problem != nil || intro.State != contracts.CommandObserved && intro.State != contracts.CommandDelivered {
		return e
	}
	ownEmail := ""
	for _, handle := range own {
		if strings.Contains(handle, "@") {
			ownEmail = handle
			break
		}
	}
	handles := []string{remy, ownEmail}
	service := "iMessage"
	if s.ID != "MSG-GROUP-MANAGE" {
		handles = []string{contacts["REMY_IMESSAGE_TEST_HANDLE"], contacts["BEN_GROUP_TEST_HANDLE"]}
		service = "RCS"
		if s.ID == "MSG-GROUP-CREATE" {
			service = "SMS"
		}
	}
	add("approved_group_participants", handles[0] != "" && handles[1] != "")
	if handles[1] == "" {
		return e
	}
	started := time.Now()
	result, problem := submit("create_group", map[string]any{"handles": handles, "service": service, "text": tag + " automated group test"})
	observed := problem == nil && (result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered)
	add("native_group_create_observed", observed)
	e.Metrics = map[string]any{"command_state": result.State, "requested_service": service}
	var nativeResult map[string]any
	json.Unmarshal(result.Evidence, &nativeResult)
	e.Metrics["apple_event_error"] = nativeResult["apple_event_error"]
	e.Metrics["native_reason"] = nativeResult["reason"]
	e.Metrics["native_stage_error"] = nativeResult["native_error_code"]
	if problem != nil {
		e.Metrics["error"] = problem.Error()
	}
	var proof struct {
		NativeKey    string `json:"native_key"`
		ChatKey      string `json:"chat_key"`
		CreatedByRun bool   `json:"created_by_run"`
	}
	json.Unmarshal(result.Evidence, &proof)
	if proof.NativeKey != "" && proof.ChatKey != "" {
		truth, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": proof.NativeKey, "expected_text": tag + " automated group test", "chat_guid": proof.ChatKey})
		add("appletruth_group_participants", problem == nil && truth["group"] == true)
		e.Metrics["native_group_service"] = truth["service"]
		e.Metrics["native_group_other_handles"] = truth["participant_count"]
		if s.ID == "MSG-GROUP-MANAGE" && truth["group"] == false {
			count, _ := truth["participant_count"].(json.Number)
			actual, _ := count.Int64()
			if problem == nil && actual < 2 {
				e.Status = "environment-limited"
				e.Metrics["management_limit"] = "Apple did not retain the own iCloud email as a second other participant; add/rename require two others and removal requires three"
			}
		}
		if s.ID == "MSG-SEND-RCS-GROUP" {
			add("appletruth_rcs_group", truth["service"] == "RCS")
		}
		if s.ID == "MSG-GROUP-MANAGE" && proof.CreatedByRun {
			state, err := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?mode=ro")
			if err != nil {
				add("group_captured", false)
				return e
			}
			defer state.Close()
			var id string
			add("group_captured", w.Await(20*time.Second, func() bool {
				return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.chat' AND native_key=?", proof.ChatKey).Scan(&id) == nil
			}) == nil)
			if id != "" {
				renamed, err := submit("rename_group", map[string]any{"id": id, "name": "Bridge Test " + e.RunID})
				add("rename_observed", err == nil && (renamed.State == contracts.CommandObserved || renamed.State == contracts.CommandDelivered))
			}
		}
	}
	report := filepath.Join(os.Getenv("HOME"), "Library/Application Support/AppleBridgeDev", "group-current-"+e.RunID+".json")
	probe := []string{"-n", "-W", "-g", "-a", w.HelperApp, "--args", "probe", "ax-dump", "--current", "--private-values", "--report", report}
	if observed {
		probe = append(probe, "--details")
	}
	exec.CommandContext(ctx, "open", probe...).Run()
	if data, problem := os.ReadFile(report); problem == nil {
		var capture map[string]any
		json.Unmarshal(data, &capture)
		var inspect func(map[string]any)
		inspect = func(node map[string]any) {
			if node["AXIdentifier"] == "messageBodyField" {
				placeholder, _ := node["AXPlaceholderValue"].(string)
				for _, service := range []string{"iMessage", "RCS", "SMS"} {
					if strings.Contains(placeholder, service) {
						e.Metrics["offered_service"] = service
					}
				}
			}
			children, _ := node["children"].([]any)
			for _, raw := range children {
				child, _ := raw.(map[string]any)
				inspect(child)
			}
		}
		root, _ := capture["tree"].(map[string]any)
		inspect(root)
	}
	if s.ID == "MSG-GROUP-CREATE" && !observed && e.Metrics["offered_service"] == "SMS" {
		e.Status = "environment-limited"
		e.Metrics["group_limit"] = "Approved participants offer SMS only; the tagged SMS group text remained in the composer and no group send was observed. An iMessage group requires two eligible other participants."
		for i := range e.Assertions {
			if e.Assertions[i].Name == "native_group_create_observed" {
				e.Assertions[i].Status = "unverified"
				e.Assertions[i].Detail = "Approved-contact SMS-only group capability is unavailable on this Mac; no group send claimed."
			}
		}
	}
	e.Latency["create_group_submit_to_truth"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	state, stateErr := sql.Open("sqlite", "file:"+filepath.Join(w.Gateway.Home, "state.db")+"?mode=ro")
	if stateErr == nil {
		defer state.Close()
		var notice struct {
			NativeKey string `json:"native_key"`
		}
		json.Unmarshal(intro.Evidence, &notice)
		add("remy_notice_unsent", notice.NativeKey != "" && unsendNativeTest(ctx, w, state, notice.NativeKey))
		if service == "iMessage" && proof.NativeKey != "" {
			add("imessage_group_seed_unsent", unsendNativeTest(ctx, w, state, proof.NativeKey))
		}
	} else {
		add("notice_cleanup_database", false)
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
