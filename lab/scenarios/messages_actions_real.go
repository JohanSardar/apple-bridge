package scenarios

import (
	"context"
	"database/sql"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runNativeMessageAction(ctx context.Context, s Scenario, e Evidence, w *world.World, native, text string, state *sql.DB) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "action-ax-"+e.RunID+".json")
	defer os.Remove(report)
	probeArgs := []string{"-n", "-W", "-a", w.HelperApp, "--args", "probe", "ax-dump", "--report", report, "--message-guid", native}
	if s.ID == "MSG-REPLY" {
		probeArgs = append(probeArgs, "--approved-contact", "remy")
	}
	err := exec.CommandContext(ctx, "open", probeArgs...).Run()
	add("gui_session_probe", err == nil)
	var id string
	add("target_captured", w.Await(30*time.Second, func() bool {
		return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=?", native).Scan(&id) == nil
	}) == nil)
	if id == "" {
		return e
	}
	op := strings.ToLower(strings.TrimPrefix(s.ID, "MSG-"))
	args := map[string]any{"id": id}
	switch op {
	case "reply":
		args["text"] = "[ab-test " + e.RunID + "] reply"
	case "edit":
		args["text"] = text + " edited"
	case "react":
		args["reaction"] = "love"
	case "unsend":
	default:
		add("native_action_implemented", false)
		return e
	}
	raw, _ := json.Marshal(args)
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages." + op, Args: raw, WaitMS: 15000}
	client := scripted.Client{Endpoint: w.Gateway}
	if op == "edit" || op == "unsend" {
		var r contracts.ReadResult
		client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: id}, &r)
		if len(r.Records) == 1 {
			command.ExpectedVersion = r.Records[0].Version
		}
	}
	diagnostic := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "messages-real-session-active.json")
	pid, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	active, _ := json.Marshal(map[string]any{"world": w.Dir, "pid": strings.TrimSpace(string(pid)), "command": command.ID})
	os.WriteFile(diagnostic, active, 0600)
	started := time.Now()
	var result contracts.CommandRecord
	err = client.Call(ctx, command.Op, command, &result)
	initialErr := err
	if err != nil || result.State == contracts.CommandDispatched || result.State == contracts.CommandPrepared || result.State == contracts.CommandAdmitted {
		w.Await(25*time.Second, func() bool {
			if client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &result) != nil {
				return initialErr != nil
			}
			return result.State != contracts.CommandAdmitted && result.State != contracts.CommandPrepared && result.State != contracts.CommandDispatched
		})
		if result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered {
			err = nil
		}
	}

	e.Metrics = map[string]any{"command_state": result.State, "disposition": result.Disposition}
	if err != nil {
		e.Metrics["error"] = initialErr.Error()
	}
	var nativeEvidence map[string]any
	json.Unmarshal(result.Evidence, &nativeEvidence)
	e.Metrics["native_reason"] = nativeEvidence["reason"]
	e.Metrics["native_stage_error"] = nativeEvidence["native_error_code"]
	if result.Error != nil {
		e.Metrics["native_error_code"] = result.Error.Code
	}
	add("native_action_observed", err == nil && (result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered))
	proof, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": native, "expected_text": args["text"]})
	add("appletruth_target_observed", problem == nil && proof["status"] == "verified")
	truthMatched := false
	rows, _ := proof["records"].([]any)
	for _, raw := range rows {
		row, _ := raw.(map[string]any)
		number := func(k string) int64 { v, _ := row[k].(json.Number); n, _ := v.Int64(); return n }
		switch op {
		case "reply":
			truthMatched = truthMatched || (row["is_from_me"] == true && row["thread_originator_guid"] == native && row["expected_text_present"] == true)
		case "react":
			truthMatched = truthMatched || (strings.HasSuffix(fmt.Sprint(row["associated_message_guid"]), native) && number("associated_message_type") == 2000)
		case "edit":
			truthMatched = truthMatched || (row["guid"] == native && number("date_edited") > 0 && row["expected_text_present"] == true && number("summary_bytes") > 0)
		case "unsend":
			truthMatched = truthMatched || (row["guid"] == native && (number("date_retracted") > 0 || row["summary_unsent"] == true && number("retracted_part_count") > 0))
		}
	}
	add("appletruth_action_effect", truthMatched)
	e.Latency["command_submit_to_truth"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	if op == "react" && truthMatched {
		cases := []struct {
			name, kind, emoji string
			remove            bool
			code              int64
		}{
			{"remove_love", "love", "", true, 3000},
			{"add_emoji", "emoji", "😂", false, 2006},
			{"remove_emoji", "emoji", "😂", true, 3006},
		}
		for _, item := range cases {
			args := map[string]any{"id": id, "reaction": item.kind, "remove": item.remove}
			if item.emoji != "" {
				args["emoji"] = item.emoji
			}
			raw, _ := json.Marshal(args)
			command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.react", Args: raw, WaitMS: 15000}
			started := time.Now()
			var result contracts.CommandRecord
			problem := client.Call(ctx, command.Op, command, &result)
			add(item.name+"_observed", problem == nil && (result.State == contracts.CommandObserved || result.State == contracts.CommandDelivered))
			proof, problem := w.Truth("observe", map[string]any{"source": "messages", "guid": native})
			matched := false
			rows, _ := proof["records"].([]any)
			for _, value := range rows {
				row, _ := value.(map[string]any)
				number, _ := row["associated_message_type"].(json.Number)
				code, _ := number.Int64()
				matched = matched || (code == item.code && strings.HasSuffix(fmt.Sprint(row["associated_message_guid"]), native) && (item.kind != "emoji" || item.remove || row["emoji"] == item.emoji))
			}
			add(item.name+"_appletruth", problem == nil && matched)
			e.Latency[item.name+"_submit_to_truth"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
			if !matched {
				break
			}
		}
	}

	if op == "reply" {
		cleanup, _ := w.Truth("observe", map[string]any{"source": "messages", "tag": "[ab-test " + e.RunID + "]"})
		rows, _ := cleanup["records"].([]any)
		index := 0
		for _, raw := range rows {
			row, _ := raw.(map[string]any)
			if row["is_from_me"] != true {
				continue
			}
			key, _ := row["guid"].(string)
			n, _ := row["retracted_part_count"].(json.Number)
			count, _ := n.Int64()
			if count > 0 {
				continue
			}
			add(fmt.Sprintf("remy_test_%d_unsent", index), unsendNativeTest(ctx, w, state, key))
			index++
		}
	}

	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	if !e.Passed {
		private := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "action-diagnostics", e.RunID)
		os.MkdirAll(private, 0700)
		for _, path := range []string{filepath.Join(w.Dir, "gateway.log"), filepath.Join(w.Gateway.Home, "helper", "dispatch-guard.log"), filepath.Join(w.Gateway.Home, "helper", "session-timers.jsonl")} {
			if data, problem := os.ReadFile(path); problem == nil {
				os.WriteFile(filepath.Join(private, filepath.Base(path)), data, 0600)
			}
		}
	}
	return e
}

func unsendNativeTest(ctx context.Context, w *world.World, state *sql.DB, native string) bool {
	var id string
	if w.Await(10*time.Second, func() bool {
		return state.QueryRowContext(ctx, "SELECT record_id FROM aliases WHERE collection='messages.message' AND native_key=?", native).Scan(&id) == nil
	}) != nil {
		return false
	}
	client := scripted.Client{Endpoint: w.Gateway}
	var read contracts.ReadResult
	if client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.message", ID: id}, &read) != nil || len(read.Records) != 1 {
		return false
	}
	raw, _ := json.Marshal(map[string]any{"id": id})
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.unsend", Args: raw, ExpectedVersion: read.Records[0].Version, WaitMS: 15000}
	var result contracts.CommandRecord
	if client.Call(ctx, command.Op, command, &result) != nil {
		return false
	}
	if result.State == contracts.CommandAdmitted || result.State == contracts.CommandPrepared || result.State == contracts.CommandDispatched {
		w.Await(20*time.Second, func() bool {
			if client.Call(ctx, "commands.status", contracts.CommandQuery{ID: command.ID}, &result) != nil {
				return false
			}
			return result.State != contracts.CommandAdmitted && result.State != contracts.CommandPrepared && result.State != contracts.CommandDispatched
		})
	}
	if result.State != contracts.CommandObserved && result.State != contracts.CommandDelivered {
		return false
	}
	proof, err := w.Truth("observe", map[string]any{"source": "messages", "guid": native})
	rows, _ := proof["records"].([]any)
	for _, raw := range rows {
		row, _ := raw.(map[string]any)
		n, _ := row["date_retracted"].(json.Number)
		value, _ := n.Int64()
		parts, _ := row["retracted_part_count"].(json.Number)
		count, _ := parts.Int64()
		if err == nil && row["guid"] == native && (value > 0 || row["summary_unsent"] == true && count > 0) {
			return true
		}
	}
	return false
}
