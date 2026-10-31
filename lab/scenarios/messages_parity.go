package scenarios

import (
	"context"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	bridgeruntime "github.com/JohanSardar/apple-bridge/internal/runtime"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

func runMessagesParity(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if level == "real" {
		return runMessagesParityProof(ctx, s, e)
	}
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "messages:read,messages:write,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	add("seed", w.ControlAction("/messages", map[string]any{"action": "seed", "count": 0}) == nil)
	add("capture", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	var read contracts.ReadResult
	client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.chat", Limit: 10}, &read)
	if len(read.Records) == 0 {
		add("chat_exists", false)
		return e
	}
	chat := read.Records[0]
	refresh := func() {
		var r contracts.ReadResult
		client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.chat", ID: chat.ID}, &r)
		if len(r.Records) > 0 {
			chat = r.Records[0]
		}
	}
	execute := func(op string, extra map[string]any) contracts.CommandRecord {
		args := map[string]any{"id": chat.ID}
		for k, v := range extra {
			args[k] = v
		}
		raw, _ := json.Marshal(args)
		c := contracts.Command{ID: contracts.NewCommandID(), Op: "messages." + op, Args: raw, ExpectedVersion: chat.Version, WaitMS: 15000}
		var r contracts.CommandRecord
		err := client.Call(ctx, c.Op, c, &r)
		add(op+"_observed", err == nil && r.State == contracts.CommandObserved && r.CommitSeq > 0)
		var replay contracts.CommandRecord
		err = client.Call(ctx, c.Op, c, &replay)
		effects, _ := w.Effects()
		add(op+"_exactly_once", err == nil && effects[c.ID] == 1)
		refresh()
		return r
	}
	fields := func() map[string]any { var d map[string]any; json.Unmarshal(chat.Data, &d); return d }
	switch s.ID {
	case "MSG-PINS":
		execute("pin", map[string]any{"pinned_index": 0})
		add("pinned", fields()["pinned_index"] == float64(0))
		execute("pin", map[string]any{"pinned_index": -1})
		add("unpinned", fields()["pinned_index"] == nil)
	case "MSG-MUTE":
		execute("mute", map[string]any{"muted": true})
		add("muted_without_false_expiry", fields()["muted"] == true && fields()["muted_until"] == nil)
		var summary contracts.ChatsResult
		problem := client.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &summary)
		known := false
		for _, value := range summary.Chats {
			if value.Chat.ID == chat.ID {
				known = value.Muted != nil && *value.Muted
			}
		}
		add("known_mute_read_model", problem == nil && known)
		execute("mute", map[string]any{"muted": false})
		add("unmuted", fields()["muted"] == false)
	case "MSG-READ-RECEIPTS":
		for _, value := range []string{"on", "off", "default"} {
			execute("set_read_receipts", map[string]any{"read_receipts": value})
			add(value, fields()["read_receipts"] == value)
		}
	case "MSG-RECENTLY-DELETED":
		aliases, _ := gatewayAliases(w)
		observations, _ := w.Inspect()
		for _, o := range observations {
			if o.Collection == "messages.chat" && aliases[aliasKey(contracts.Messages, o.Collection, o.NativeKey)] == chat.ID {
				var d map[string]any
				json.Unmarshal(o.Data, &d)
				var other map[string]any
				json.Unmarshal(o.Data, &other)
				other["display_name"] = "[ab-test] preserved conversation"
				add("other_fixture", w.ControlAction("/external-change", map[string]any{"key": "parity-preserved-chat", "collection": "messages.chat", "data": other}) == nil)
				add("other_captured", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
				aliasesAfter, _ := gatewayAliases(w)
				otherID := aliasesAfter[aliasKey(contracts.Messages, "messages.chat", "parity-preserved-chat")]
				d["deleted_at"] = time.Now().UTC().Format(time.RFC3339Nano)
				d["recoverable_until"] = nil
				add("delete_fixture", w.ControlAction("/external-change", map[string]any{"key": o.NativeKey, "collection": o.Collection, "data": d}) == nil)
				add("deleted_capture", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
				refresh()
				execute("recover_chat", nil)
				add("recovered", fields()["deleted_at"] == nil)
				d["deleted_at"] = time.Now().UTC().Format(time.RFC3339Nano)
				w.ControlAction("/external-change", map[string]any{"key": o.NativeKey, "collection": o.Collection, "data": d})
				w.Await(5*time.Second, func() bool { return convergence(w) })
				refresh()
				result := execute("delete_chat_permanently", nil)
				add("purge_identity", len(result.ObservedRecordIDs) > 0)
				var gone contracts.ReadResult
				client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.chat", ID: chat.ID}, &gone)
				add("purged", len(gone.Records) == 0 || gone.Records[0].Deleted)
				var preserved contracts.ReadResult
				problem := client.Call(ctx, "records.get", contracts.ReadQuery{Collection: "messages.chat", ID: otherID}, &preserved)
				add("other_conversation_preserved", problem == nil && len(preserved.Records) == 1 && !preserved.Records[0].Deleted)
				break
			}
		}
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func runMessagesParityProof(ctx context.Context, s Scenario, e Evidence) Evidence {
	app := os.Getenv("APPLE_BRIDGE_PARITY_HELPER_APP")
	if app == "" {
		home, err := os.MkdirTemp("", "ab-messages-parity-")
		if err != nil {
			e.Assertions = []Assertion{{Name: "helper_directory", Passed: false}}
			return e
		}
		defer os.RemoveAll(home)
		app = filepath.Join(home, "AppleBridgeHelper.app")
		if err := exec.CommandContext(ctx, "packaging/dev/build-helper.sh", app).Run(); err != nil {
			e.Assertions = []Assertion{{Name: "signed_parity_helper", Passed: false}}
			return e
		}
	}
	data, _ := bridgeruntime.MessagesParitySelfTest(ctx, app, os.Getenv("APPLE_BRIDGE_PARITY_FORCE") == "1", true)
	var report struct {
		OSBuild          string `json:"os_build"`
		Cleanup          string `json:"cleanup_state"`
		Writes           int    `json:"writes_submitted"`
		Before           int    `json:"before_count"`
		After            int    `json:"after_count"`
		Preserved        bool   `json:"baseline_preserved"`
		BaselineCaptured bool   `json:"baseline_captured"`
		Steps            []struct {
			Operation string `json:"operation"`
			Proven    bool   `json:"proven"`
			Reason    string `json:"reason"`
		} `json:"steps"`
	}
	if json.Unmarshal(data, &report) != nil || report.OSBuild == "" {
		e.Assertions = []Assertion{{Name: "native_parity_report", Passed: false}}
		return e
	}
	want := map[string][]string{"MSG-PINS": {"messages.pin"}, "MSG-MUTE": {"messages.mute"}, "MSG-READ-RECEIPTS": {"messages.set_read_receipts"}, "MSG-RECENTLY-DELETED": {"messages.recover_chat", "messages.delete_chat_permanently"}}[s.ID]
	e.Metrics = map[string]any{"writes_submitted": report.Writes, "baseline_captured": report.BaselineCaptured, "cleanup_verified": report.Cleanup == "verified_clean" || report.Cleanup == "not_needed"}
	if report.BaselineCaptured {
		e.Metrics["before_count"] = report.Before
		e.Metrics["after_count"] = report.After
		e.Metrics["baseline_preserved"] = report.Preserved
	}
	e.Passed = report.Cleanup == "verified_clean" && report.BaselineCaptured && report.Preserved
	e.Assertions = append(e.Assertions, Assertion{Name: "run_only_cleanup", Passed: report.Cleanup == "verified_clean" || report.Cleanup == "not_needed"})
	for _, op := range want {
		found := false
		for _, step := range report.Steps {
			if step.Operation == op {
				found = true
				e.Assertions = append(e.Assertions, Assertion{Name: op, Passed: step.Proven, Detail: step.Reason})
				e.Passed = e.Passed && step.Proven
			}
		}
		if !found {
			e.Passed = false
			e.Assertions = append(e.Assertions, Assertion{Name: op, Passed: false, Detail: "variant_not_proven"})
		}
	}
	if !e.Passed {
		e.Status = "environment-limited"
	}
	return e
}
