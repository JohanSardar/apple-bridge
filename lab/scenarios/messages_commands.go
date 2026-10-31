package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"strings"
	"time"
)

func runMessagesCommands(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level == "real" && messageSendVariant(s.ID) != "" {
		return runMessagesVariantReal(ctx, s, e)
	}
	if s.ID == "MSG-SELFTEST" && level == "real" {
		return runNativeMessagesSelfTest(ctx, e)
	}
	if level == "real" && s.ID == "MSG-RESEND" && os.Getenv("APPLE_BRIDGE_RESEND_RUN") != e.RunID {
		e.Status = "environment-limited"
		e.Metrics = map[string]any{"writes_submitted": 0}
		e.Assertions = append(e.Assertions, Assertion{Name: "safe_failed_self_fixture", Status: "unverified", Detail: "This check needs a failed self-message created and registered by the current run. No native failure was induced; existing failed messages were left untouched."})
		return e
	}
	if level == "real" && (s.ID == "MSG-READSTATE-REAL" || s.ID == "MSG-DELETE-REAL") && os.Getenv("APPLE_BRIDGE_MESSAGES_UI_DEFERRED") == "1" {
		e.Status = "environment-limited"
		e.Metrics = map[string]any{"writes_submitted": 0, "ui_deferred": true}
		e.Assertions = append(e.Assertions, Assertion{Name: "unlocked_messages_window", Status: "unverified", Detail: "UI proof deferred until the Air is unlocked; no world or command was created"})
		return e
	}
	if level != "sim" {
		return runNativeMessagesCommands(ctx, s, level, e)
	}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if kind := messageSendVariant(s.ID); kind != "" {
		runMessagesVariantReadback(ctx, kind, add)
	}
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "messages:read,messages:write,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	seedCount := 0
	if s.ID == "MSG-SELFTEST" || s.ID == "MSG-READSTATE-REAL" || s.ID == "MSG-DELETE-REAL" {
		seedCount = 1
	}
	add("seed_self_chat", w.ControlAction("/messages", map[string]any{"action": "seed", "count": seedCount}) == nil)
	add("fixture_captured", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	read := func(collection string) []contracts.Record {
		var r contracts.ReadResult
		client.Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, Limit: 100}, &r)
		return r.Records
	}
	baselineMessages := read("messages.message")
	preserved := func() bool {
		current := read("messages.message")
		missing, changed := 0, 0
		for _, before := range baselineMessages {
			found := false
			for _, after := range current {
				if before.ID == after.ID {
					found = true
					if before.Version != after.Version {
						changed++
					}
				}
			}
			if !found {
				missing++
			}
		}
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		e.Metrics["baseline_missing"] = missing
		e.Metrics["baseline_changed"] = changed
		e.Metrics["remaining_messages"] = len(current)
		e.Metrics["baseline_messages"] = len(baselineMessages)
		return len(current) == len(baselineMessages) && missing == 0 && changed == 0
	}
	chats := read("messages.chat")
	if len(chats) == 0 {
		add("self_chat", false)
		return e
	}
	chat := chats[0]
	command := func(op string, args map[string]any, version uint64) contracts.Command {
		raw, _ := json.Marshal(args)
		return contracts.Command{ID: contracts.NewCommandID(), Op: "messages." + op, Args: raw, ExpectedVersion: version, WaitMS: 15000}
	}
	submit := func(c contracts.Command) (contracts.CommandRecord, error) {
		var r contracts.CommandRecord
		err := client.Call(ctx, c.Op, c, &r)
		return r, err
	}
	count := func(c contracts.Command, n int) bool {
		effects, err := w.Effects()
		return err == nil && effects[c.ID] == n
	}
	fault := func(name string) { add("fault_armed", w.ControlAction("/fault", map[string]any{"name": name}) == nil) }
	text := "[ab-test " + e.RunID + "] message"
	send := func() (contracts.CommandRecord, error) {
		return submit(command("send", map[string]any{"chat_id": chat.ID, "text": text}, 0))
	}
	observed := func(r contracts.CommandRecord, err error) bool {
		return err == nil && r.State == contracts.CommandObserved && r.CommitSeq > 0
	}
	record := func(collection, id string) contracts.Record {
		var r contracts.ReadResult
		client.Call(ctx, "records.get", contracts.ReadQuery{Collection: collection, ID: id}, &r)
		if len(r.Records) > 0 {
			return r.Records[0]
		}
		return contracts.Record{}
	}
	findMessage := func() contracts.Record {
		for _, r := range read("messages.message") {
			var d map[string]any
			json.Unmarshal(r.Data, &d)
			if d["is_from_me"] == true && d["text"] == text {
				return r
			}
		}
		return contracts.Record{}
	}
	target := func(op string, r contracts.Record, extras map[string]any) contracts.Command {
		args := map[string]any{"id": r.ID}
		for k, v := range extras {
			args[k] = v
		}
		version := uint64(0)
		if op == "edit" || op == "unsend" || strings.HasPrefix(op, "delete_") {
			version = r.Version
		}
		return command(op, args, version)
	}
	mutateTarget := func(r contracts.Record, patch map[string]any) contracts.Record {
		aliases, _ := gatewayAliases(w)
		observations, _ := w.Inspect()
		for _, o := range observations {
			if o.Collection == r.Collection && aliases[aliasKey(contracts.Messages, o.Collection, o.NativeKey)] == r.ID {
				var data map[string]any
				json.Unmarshal(o.Data, &data)
				for k, v := range patch {
					data[k] = v
				}
				add("external_native_mutation", w.ControlAction("/external-change", map[string]any{"key": o.NativeKey, "collection": o.Collection, "data": data}) == nil)
				break
			}
		}
		add("native_mutation_captured", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
		return record(r.Collection, r.ID)
	}

	switch s.ID {
	case "MSG-SELFTEST":
		ownedIDs := map[string]bool{}
		own := func(result contracts.CommandRecord) {
			for _, id := range result.ObservedRecordIDs {
				ownedIDs[id] = true
			}
		}
		created, problem := send()
		own(created)
		add("send_text", observed(created, problem))
		msg := findMessage()
		if msg.ID == "" {
			add("owned_self_message", false)
			return e
		}
		file := command("send", map[string]any{"chat_id": chat.ID, "files": []contracts.MessageFile{{Name: "ab-" + e.RunID + ".txt", Data: base64.StdEncoding.EncodeToString([]byte(text))}}}, 0)
		result, problem := submit(file)
		own(result)
		add("send_file", observed(result, problem))
		result, problem = submit(target("reply", msg, map[string]any{"text": text + " reply"}))
		own(result)
		add("reply", observed(result, problem))
		for _, kind := range []string{"love", "emoji"} {
			for _, remove := range []bool{false, true} {
				result, problem = submit(target("react", msg, map[string]any{"reaction": kind, "emoji": "🧪", "remove": remove}))
				add(kind+fmtBool(remove), observed(result, problem))
			}
		}
		msg = record("messages.message", msg.ID)
		result, problem = submit(target("edit", msg, map[string]any{"text": text + " edited"}))
		add("edit", observed(result, problem))
		for _, op := range []string{"mark_unread", "mark_read"} {
			result, problem = submit(target(op, chat, nil))
			add(op, observed(result, problem))
		}
		unsend, problem := send()
		own(unsend)
		add("unsend_fixture", observed(unsend, problem) && len(unsend.ObservedRecordIDs) == 1)
		if len(unsend.ObservedRecordIDs) == 1 {
			fixture := record("messages.message", unsend.ObservedRecordIDs[0])
			result, problem = submit(target("unsend", fixture, nil))
			add("unsend", observed(result, problem))
		}
		for _, message := range read("messages.message") {
			var data struct {
				Text string `json:"text"`
			}
			json.Unmarshal(message.Data, &data)
			if !ownedIDs[message.ID] && !strings.HasPrefix(data.Text, "[ab-test "+e.RunID+"]") {
				continue
			}
			result, problem = submit(target("delete_message", message, nil))
			add("owned_message_cleanup", observed(result, problem))
		}
		remaining := read("messages.message")
		e.Metrics = map[string]any{"remaining_messages": len(remaining), "baseline_messages": len(baselineMessages)}
		add("existing_residue_preserved", preserved())
		add("self_chat_preserved", record("messages.chat", chat.ID).ID == chat.ID)
	case "MSG-COMMAND-PARTS":
		checkMessageParts(ctx, w, &e, chat.ID, add)
	case "MSG-FAILED-OBSERVED-IDS":
		checkFailedMessageIDs(ctx, w, &e, chat.ID, add)
	case "MSG-COMMAND-OBSERVED-IDS":
		checkMessageCommandIDs(ctx, w, &e, chat, add)
	case "MSG-RESEND":
		fault("delivery-failed")
		original := command("send", map[string]any{"chat_id": chat.ID, "text": text}, 0)
		failed, problem := submit(original)
		add("deterministic_delivery_failure", problem == nil && failed.State == contracts.CommandDeliveryFailed && count(original, 1))
		if failed.State != contracts.CommandDeliveryFailed {
			return e
		}
		message := findMessage()
		resend := command("send", map[string]any{"chat_id": chat.ID, "text": text, "resend_of": message.ID}, 0)
		result, problem := submit(resend)
		add("explicit_resend_is_new_observed_effect", observed(result, problem) && count(resend, 1) && count(original, 1))
		retry, problem := submit(resend)
		add("resend_command_replay_is_idempotent", problem == nil && retry.CreatedAt.Equal(result.CreatedAt) && count(resend, 1))
		wrong := command("send", map[string]any{"chat_id": chat.ID, "text": text, "resend_of": "missing"}, 0)
		_, problem = submit(wrong)
		add("resend_missing_target_rejected", problem != nil && count(wrong, 0))
		var successful contracts.Record
		for _, item := range read("messages.message") {
			var data map[string]any
			json.Unmarshal(item.Data, &data)
			if data["is_from_me"] == true && data["error_code"] == float64(0) {
				successful = item
				break
			}
		}
		wrong = command("send", map[string]any{"chat_id": chat.ID, "text": text, "resend_of": successful.ID}, 0)
		_, problem = submit(wrong)
		add("resend_successful_target_rejected", successful.ID != "" && problem != nil && count(wrong, 0))

	case "MSG-SEND-VARIANTS", "MSG-SEND-LATER", "MSG-SEND-EFFECT", "MSG-FORMATTING-SEND", "MSG-MENTION-SEND", "MSG-AUDIO-MESSAGE":
		var sources contracts.ReadResult
		add("source_status", client.Call(ctx, "sources.status", struct{}{}, &sources) == nil)
		coverage := map[string]contracts.OperationCapability{}
		for _, source := range sources.Records {
			var data struct {
				Coverage     contracts.Coverage `json:"coverage"`
				Capabilities []string           `json:"capabilities"`
			}
			json.Unmarshal(source.Data, &data)
			for _, item := range data.Coverage.Operations {
				coverage[item.Operation] = item
			}
			for _, name := range contracts.MessageVariantCapabilities {
				advertised := false
				for _, capability := range data.Capabilities {
					advertised = advertised || capability == name
				}
				add("not_advertised_"+name, !advertised)
			}
		}
		for _, name := range contracts.MessageVariantCapabilities {
			proof, ok := coverage[name]
			add("limited_reason_"+name, ok && !proof.Proven && proof.Reason == "per_os_variant_proof_required")
		}
		variants := []map[string]any{
			{"kind": "schedule", "scheduled_for": time.Now().Add(time.Hour).UTC().Format(time.RFC3339Nano)},
			{"kind": "effect", "effect": "loud"},
			{"kind": "formatting", "formatting": []any{map[string]any{"utf16_location": 0, "utf16_length": 1, "styles": []string{"bold"}}}},
			{"kind": "mention", "handle_id": "handle-self", "utf16_location": 0, "utf16_length": 1},
			{"kind": "audio"},
		}
		selected := map[string]string{"MSG-SEND-LATER": "schedule", "MSG-SEND-EFFECT": "effect", "MSG-FORMATTING-SEND": "formatting", "MSG-MENTION-SEND": "mention", "MSG-AUDIO-MESSAGE": "audio"}[s.ID]
		for _, variant := range variants {
			if selected != "" && variant["kind"] != selected {
				continue
			}
			args := map[string]any{"chat_id": chat.ID, "text": text, "variant": variant}
			if variant["kind"] == "audio" {
				args["text"] = ""
				args["files"] = []any{map[string]any{"name": "ab-test-audio.wav", "data": base64.StdEncoding.EncodeToString([]byte("synthetic audio"))}}
			}
			c := command("send", args, 0)
			_, problem := submit(c)
			failure, ok := problem.(*contracts.Error)
			add("unproven_"+variant["kind"].(string)+"_no_plain_fallback", ok && failure.Code == "capability_unproven" && count(c, 0))
		}
		invalid := []map[string]any{
			{"kind": "unknown"},
			{"kind": "schedule", "scheduled_for": time.Now().Add(-time.Hour).UTC().Format(time.RFC3339Nano)},
			{"kind": "effect", "effect": "invented"},
			{"kind": "effect", "effect": "loud", "scheduled_for": time.Now().Add(time.Hour).UTC().Format(time.RFC3339Nano)},
			{"kind": "formatting", "formatting": []any{map[string]any{"utf16_location": 2, "utf16_length": 1, "styles": []string{"bold"}}}},
			{"kind": "formatting", "formatting": []any{map[string]any{"utf16_location": 0, "utf16_length": 0, "styles": []string{"bold"}}}},
			{"kind": "formatting", "formatting": []any{map[string]any{"utf16_location": 0, "utf16_length": 1, "styles": []string{"bold", "bold"}}}},
		}
		for index, variant := range invalid {
			c := command("send", map[string]any{"chat_id": chat.ID, "text": "A😀B", "variant": variant}, 0)
			_, problem := submit(c)
			failure, ok := problem.(*contracts.Error)
			add(fmt.Sprintf("invalid_variant_%d_no_effect", index), ok && failure.Code == "invalid" && count(c, 0))
		}
		ordinary, problem := send()
		add("plain_send_unchanged", observed(ordinary, problem))
		if len(ordinary.ObservedRecordIDs) > 0 {
			target := record("messages.message", ordinary.ObservedRecordIDs[0])
			c := command("keep_audio", map[string]any{"id": target.ID}, target.Version)
			_, problem = submit(c)
			failure, ok := problem.(*contracts.Error)
			add("keep_unproven_no_effect", ok && failure.Code == "capability_unproven" && count(c, 0))
		} else {
			add("keep_unproven_no_effect", false)
		}
	case "MSG-CAPABILITY-MATRIX":
		var status contracts.ReadResult
		err := client.Call(ctx, "sources.status", struct{}{}, &status)
		add("status_available", err == nil)
		operations := map[string]contracts.OperationCapability{}
		for _, record := range status.Records {
			var data struct {
				Source   string             `json:"source"`
				Coverage contracts.Coverage `json:"coverage"`
			}
			json.Unmarshal(record.Data, &data)
			if data.Source == "messages" {
				for _, operation := range data.Coverage.Operations {
					operations[operation.Operation] = operation
				}
			}
		}
		for _, op := range contracts.MessagesOperations {
			cap, ok := operations[op]
			add("capability_reason_"+op, ok && cap.Proven && cap.Reason != "")
		}
		for _, op := range contracts.MessagesOperations {
			d, ok := contracts.Descriptor(op)
			add("descriptor_"+op, ok && d.Scope == "messages:write" && len(contracts.OperationSchema(d)) > 0)
		}
		add("capability_unproven", w.ControlAction("/messages", map[string]any{"action": "capability-unproven"}) == nil)
		c := command("send", map[string]any{"chat_id": chat.ID, "text": text}, 0)
		_, err = submit(c)
		problem, ok := err.(*contracts.Error)
		add("unproven_rejects_without_effect", ok && problem.Code == "capability_unproven" && count(c, 0))
		var after contracts.ReadResult
		client.Call(ctx, "sources.status", struct{}{}, &after)
		rejectedCapabilities := 0
		for _, source := range after.Records {
			var data struct {
				Source   string             `json:"source"`
				Coverage contracts.Coverage `json:"coverage"`
			}
			json.Unmarshal(source.Data, &data)
			if data.Source == "messages" {
				for _, capability := range data.Coverage.Operations {
					if !capability.Proven && capability.Reason == "capability_unproven" {
						rejectedCapabilities++
					}
				}
			}
		}
		add("unproven_matrix_has_per_operation_reasons", rejectedCapabilities == len(contracts.MessagesOperations))
	case "MSG-SEND-GHOST", "MSG-SEND-IDENTICAL-MANUAL":
		name := "ghost-send"
		want := contracts.CommandDeliveryFailed
		if strings.Contains(s.ID, "IDENTICAL") {
			name = "identical-manual-send"
			want = contracts.CommandIndeterminate
		}
		fault(name)
		c := command("send", map[string]any{"chat_id": chat.ID, "text": text}, 0)
		r, err := submit(c)
		add("truthful_disposition", err == nil && r.State == want && count(c, 1))
		retry, retryErr := submit(c)
		add("no_automatic_or_caller_redispatch", retryErr == nil && retry.State == want && count(c, 1))
		if want == contracts.CommandIndeterminate {
			var resolved contracts.CommandRecord
			err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID, Resolve: true}, &resolved)
			add("resolve_never_guesses_identical_candidate", err == nil && resolved.State == want && count(c, 1))
		}
	case "MSG-SEND-SELF-TEXT", "MSG-SEND-SELF-EMAIL", "MSG-SEND-FILES", "MSG-SEND-NEW-HANDLE":
		for _, protocol := range []string{"rest", "mcp"} {
			client.Protocol = protocol
			args := map[string]any{"chat_id": chat.ID, "text": text + protocol}
			if s.ID == "MSG-SEND-NEW-HANDLE" || s.ID == "MSG-SEND-SELF-EMAIL" {
				delete(args, "chat_id")
				args["handles"] = []string{"second-self@example.com"}
				args["service"] = "iMessage"
			}
			if s.ID == "MSG-SEND-FILES" {
				e.Metrics = map[string]any{"largest_file_bytes": 5 * 1024 * 1024, "ipc_file_transport": "private_path_and_digest"}
				large, problem := stageMessageFile(ctx, w.Gateway, "one.txt", messageSyntheticFile(text))
				add(protocol+"_five_mib_upload_staged", problem == nil)
				if problem != nil {
					return e
				}
				args["files"] = []contracts.MessageFile{large, {Name: "two.txt", Data: base64.StdEncoding.EncodeToString([]byte("two"))}}
			}
			c := command("send", args, 0)
			started := time.Now()
			r, err := submit(c)
			add(protocol+"_locally_observed_with_checkpoint", observed(r, err) && count(c, 1))
			if r.State == contracts.CommandObserved {
				var resolved contracts.CommandRecord
				err = client.Call(ctx, "commands.status", contracts.CommandQuery{ID: c.ID, Resolve: true}, &resolved)
				add(protocol+"_delivered_only_after_native_evidence", err == nil && resolved.State == contracts.CommandDelivered)
			}
			add(protocol+"_replica_converges", w.Await(5*time.Second, func() bool { return convergence(w) }) == nil)
			e.Latency[protocol+"_command_to_replica"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
			if s.ID == "MSG-SEND-FILES" {
				var evidence struct {
					Parts []any `json:"parts"`
				}
				json.Unmarshal(r.Evidence, &evidence)
				add(protocol+"_three_part_outcomes", len(evidence.Parts) == 3)
				attachments := read("messages.attachment")
				add(protocol+"_attachment_records", len(attachments) >= 2)
				for _, attachment := range attachments {
					var data struct {
						Name string `json:"filename"`
					}
					json.Unmarshal(attachment.Data, &data)
					expected := messageSyntheticFile(text)
					if data.Name == "two.txt" {
						expected = []byte("two")
					}
					for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
						add(protocol+"_authorized_attachment_bytes", messagesAttachmentBytes(ctx, w, endpoint, attachment.ID, expected))
					}
				}
			}
		}
		bad := command("send", map[string]any{"handles": []string{"other@example.com"}, "service": "iMessage", "text": text}, 0)
		_, err := submit(bad)
		add("non_self_refused_before_effect", err != nil && count(bad, 0))
	case "MSG-GROUP-CREATE", "MSG-GROUP-MANAGE", "MSG-SEND-RCS-GROUP":
		c := command("create_group", map[string]any{"handles": []string{"self@example.com", "member1@example.com", "member2@example.com"}, "service": "iMessage", "text": text, "name": "Bridge Test"}, 0)
		r, err := submit(c)
		add("group_created", observed(r, err))
		var group contracts.Record
		for _, g := range read("messages.chat") {
			if g.ID != chat.ID {
				group = g
			}
		}
		if s.ID == "MSG-GROUP-MANAGE" && group.ID != "" {
			for _, op := range []string{"rename_group", "add_member"} {
				extra := map[string]any{"name": "Bridge Test renamed"}
				if op == "add_member" {
					extra = map[string]any{"handle": "member3@example.com"}
				}
				r, err := submit(target(op, group, extra))
				add(op, observed(r, err))
				group = record("messages.chat", group.ID)
			}
			r, err := submit(target("remove_member", group, map[string]any{"handle": "member3@example.com"}))
			add("remove_member", observed(r, err))
			group = record("messages.chat", group.ID)
			rejected := target("remove_member", group, map[string]any{"handle": "member2@example.com"})
			r, err = submit(rejected)
			add("remove_rejects_below_four_participants", err != nil && count(rejected, 0))
			r, err = submit(target("leave_group", group, nil))
			add("leave_group", observed(r, err))
		}
		for _, service := range []string{"SMS", "RCS"} {
			created := command("create_group", map[string]any{"handles": []string{"self@example.com", "second-self@example.com"}, "service": service, "text": text}, 0)
			r, err := submit(created)
			add(service+"_group_creation_route_explicit", observed(r, err))
			for _, g := range read("messages.chat") {
				var d map[string]any
				json.Unmarshal(g.Data, &d)
				if d["service"] != service {
					continue
				}
				if service == "RCS" {
					c := command("send", map[string]any{"chat_id": g.ID, "text": text + " RCS group"}, 0)
					r, err := submit(c)
					add("rcs_existing_group_send", observed(r, err))
				}
				for _, op := range []string{"rename_group", "add_member", "remove_member", "leave_group"} {
					extra := map[string]any{}
					if op == "rename_group" {
						extra["name"] = "Bridge Test renamed"
					}
					if op == "add_member" || op == "remove_member" {
						extra["handle"] = "self@example.com"
					}
					c := target(op, g, extra)
					_, err := submit(c)
					add(service+"_"+op+"_rejected_before_effect", err != nil && count(c, 0))
				}
			}
		}

	default:
		r, err := send()
		add("target_created", observed(r, err))
		msg := findMessage()
		if msg.ID == "" {
			add("target_message", false)
			return e
		}
		switch s.ID {
		case "MSG-REPLY":
			r, err = submit(target("reply", msg, map[string]any{"text": text + " reply"}))
			add("threaded_reply", observed(r, err))
			linked := false
			for _, reply := range read("messages.message") {
				var data map[string]any
				json.Unmarshal(reply.Data, &data)
				if data["thread_originator_id"] == msg.ID && data["text"] == text+" reply" {
					linked = true
				}
			}
			add("reply_points_to_exact_target", linked)
		case "MSG-REACT":
			for _, reaction := range []string{"love", "emoji"} {
				for _, remove := range []bool{false, true} {
					r, err = submit(target("react", msg, map[string]any{"reaction": reaction, "emoji": "🧪", "remove": remove}))
					add("reaction_"+reaction+fmtBool(remove), observed(r, err))
					matched := false
					for _, reactionRecord := range read("messages.reaction") {
						var data map[string]any
						json.Unmarshal(reactionRecord.Data, &data)
						if data["message_id"] == msg.ID && data["kind"] == reaction && data["removed"] == remove && (reaction != "emoji" || data["emoji"] == "🧪") {
							matched = true
						}
					}
					add("reaction_native_fields", matched)
				}
			}
		case "MSG-EDIT":
			for i := 0; i < 6; i++ {
				msg = record("messages.message", msg.ID)
				c := target("edit", msg, map[string]any{"text": text + strings.Repeat("!", i+1)})
				r, err = submit(c)
				if i < 5 {
					add("edit_within_limits", observed(r, err))
					current := record("messages.message", msg.ID)
					var data struct {
						Text    string `json:"text"`
						History []any  `json:"edit_history"`
						Parts   []struct {
							Index int    `json:"index"`
							Text  string `json:"text"`
						} `json:"parts"`
					}
					json.Unmarshal(current.Data, &data)
					add("edit_history_and_new_text", data.Text == text+strings.Repeat("!", i+1) && len(data.History) == i+1)
					add("edited_part_text_matches_capture", len(data.Parts) == 1 && data.Parts[0].Index == 0 && data.Parts[0].Text == data.Text)
				} else {
					add("sixth_edit_rejected", err != nil && count(c, 0))
				}
			}
		case "MSG-UNSEND":
			r, err = submit(target("unsend", msg, nil))
			add("unsend_within_window", observed(r, err))
			current := record("messages.message", msg.ID)
			var data map[string]any
			json.Unmarshal(current.Data, &data)
			add("native_retracted_evidence", data["retracted_at"] != nil)
		case "MSG-MARK-READ", "MSG-READSTATE-REAL":
			for _, op := range []string{"mark_unread", "mark_read"} {
				r, err = submit(target(op, chat, nil))
				add(op, observed(r, err))
				current := record("messages.chat", chat.ID)
				var data map[string]any
				json.Unmarshal(current.Data, &data)
				add("native_chat_read_state", data["is_unread"] == (op == "mark_unread"))
			}
		case "MSG-DELETE", "MSG-DELETE-REAL":
			r, err = submit(target("delete_message", msg, nil))
			add("delete_message_complete_scan", observed(r, err) && record("messages.message", msg.ID).ID == "")
			if s.ID == "MSG-DELETE-REAL" {
				for _, message := range read("messages.message") {
					var data struct {
						Text string `json:"text"`
					}
					json.Unmarshal(message.Data, &data)
					if !strings.HasPrefix(data.Text, "[ab-test "+e.RunID+"]") {
						continue
					}
					r, err = submit(target("delete_message", message, nil))
					add("owned_echo_cleanup", observed(r, err))
				}
				add("existing_residue_preserved", preserved())
			}
			if s.ID == "MSG-DELETE" {
				chat = record("messages.chat", chat.ID)
				r, err = submit(target("delete_chat", chat, nil))
				add("delete_chat_complete_scan", observed(r, err) && record("messages.chat", chat.ID).ID == "")
			}
		case "MSG-UI-INTERFERENCE":
			for _, name := range []string{"ui-interference-before", "ui-interference"} {
				fault(name)
				c := target("reply", msg, map[string]any{"text": text})
				r, err = submit(c)
				if name == "ui-interference-before" {
					var status contracts.CommandRecord
					world.Request(ctx, w.Gateway, "/v1/op/commands.status", contracts.CommandQuery{ID: c.ID}, &status)
					add(name, err != nil && status.State == contracts.CommandRejected && count(c, 0))
				} else {
					add(name, err == nil && r.State == contracts.CommandIndeterminate && count(c, 0))
				}
			}
		}
	}

	if s.ID == "MSG-EDIT" || s.ID == "MSG-UNSEND" {
		r, err := send()
		add("window_target_created", observed(r, err))
		var proof struct {
			NativeKey string `json:"native_key"`
		}
		json.Unmarshal(r.Evidence, &proof)
		aliases, _ := gatewayAliases(w)
		id := aliases[aliasKey(contracts.Messages, "messages.message", proof.NativeKey)]
		fresh := record("messages.message", id)
		op := "edit"
		age := 16 * time.Minute
		if s.ID == "MSG-UNSEND" {
			op = "unsend"
			age = 3 * time.Minute
		}
		old := mutateTarget(fresh, map[string]any{"sent_at": time.Now().Add(-age).UTC()})
		extra := map[string]any{}
		if op == "edit" {
			extra["text"] = text + " outside window"
		}
		c := target(op, old, extra)
		_, err = submit(c)
		add("outside_apple_window_rejected_before_effect", err != nil && count(c, 0))
		incoming := mutateTarget(fresh, map[string]any{"sent_at": time.Now().UTC(), "is_from_me": false})
		c = target(op, incoming, extra)
		_, err = submit(c)
		add("incoming_edit_unsend_rejected_before_effect", err != nil && count(c, 0))
	}
	add("truth_and_replica_convergence", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
func fmtBool(b bool) string {
	if b {
		return "_remove"
	}
	return "_add"
}
