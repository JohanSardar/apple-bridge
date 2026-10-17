package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os/exec"
	"strings"
	"time"
)

func runMessagesCommands(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level != "sim" {
		return runNativeMessagesCommands(ctx, s, level, e)
	}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(ctx, level, 1, 0, world.BlobOptions{Scope: "messages:read,messages:write,system:read"})
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	add("seed_self_chat", w.ControlAction("/messages", map[string]any{"action": "seed", "count": 0}) == nil)
	add("fixture_captured", w.Await(10*time.Second, func() bool { return convergence(w) }) == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	read := func(collection string) []contracts.Record {
		var r contracts.ReadResult
		client.Call(ctx, "records.query", contracts.ReadQuery{Collection: collection, Limit: 100}, &r)
		return r.Records
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
			if d["is_from_me"] == true {
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
	switch s.ID {
	case "MSG-CAPABILITY-MATRIX":
		var status contracts.ReadResult
		err := client.Call(ctx, "sources.status", struct{}{}, &status)
		add("status_available", err == nil)
		for _, op := range contracts.MessagesOperations {
			d, ok := contracts.Descriptor(op)
			add("descriptor_"+op, ok && d.Scope == "messages:write" && len(contracts.OperationSchema(d)) > 0)
		}
		add("permission_loss", w.ControlAction("/messages", map[string]any{"action": "permission-loss"}) == nil)
		c := command("send", map[string]any{"chat_id": chat.ID, "text": text}, 0)
		_, err = submit(c)
		add("unproven_rejects_without_effect", err != nil && count(c, 0))
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
				args["files"] = []contracts.MessageFile{{Name: "one.txt", Data: base64.StdEncoding.EncodeToString([]byte("one"))}, {Name: "two.txt", Data: base64.StdEncoding.EncodeToString([]byte("two"))}}
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
				add(protocol+"_attachment_blobs", len(read("messages.attachment")) >= 2)
			}
		}
		bad := command("send", map[string]any{"handles": []string{"other@example.com"}, "service": "iMessage", "text": text}, 0)
		_, err := submit(bad)
		add("non_self_refused_before_effect", err != nil && count(bad, 0))
	case "MSG-GROUP-CREATE", "MSG-GROUP-MANAGE":
		c := command("create_group", map[string]any{"handles": []string{"self@example.com", "second-self@example.com"}, "service": "iMessage", "text": text, "name": "Bridge Test"}, 0)
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
					extra = map[string]any{"handle": "self@example.com"}
				}
				r, err := submit(target(op, group, extra))
				add(op, observed(r, err))
				group = record("messages.chat", group.ID)
			}
			r, err := submit(target("remove_member", group, map[string]any{"handle": "self@example.com"}))
			add("remove_rejects_below_four_participants", err != nil && r.State == contracts.CommandRejected)
			r, err = submit(target("leave_group", group, nil))
			add("leave_group", observed(r, err))
		}
		bad := command("create_group", map[string]any{"handles": []string{"self@example.com", "second-self@example.com"}, "service": "SMS", "text": text}, 0)
		_, err = submit(bad)
		add("sms_group_creation_route_explicit", err == nil)
		for _, g := range read("messages.chat") {
			var d map[string]any
			json.Unmarshal(g.Data, &d)
			if d["service"] == "SMS" {
				c := target("add_member", g, map[string]any{"handle": "self@example.com"})
				_, err = submit(c)
				add("sms_membership_rejected_before_effect", err != nil && count(c, 0))
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
		case "MSG-REACT":
			for _, reaction := range []string{"love", "emoji"} {
				for _, remove := range []bool{false, true} {
					r, err = submit(target("react", msg, map[string]any{"reaction": reaction, "emoji": "🧪", "remove": remove}))
					add("reaction_"+reaction+fmtBool(remove), observed(r, err))
				}
			}
		case "MSG-EDIT":
			for i := 0; i < 6; i++ {
				msg = record("messages.message", msg.ID)
				c := target("edit", msg, map[string]any{"text": text + strings.Repeat("!", i+1)})
				r, err = submit(c)
				if i < 5 {
					add("edit_within_limits", observed(r, err))
				} else {
					add("sixth_edit_rejected", err != nil && count(c, 0))
				}
			}
		case "MSG-UNSEND":
			r, err = submit(target("unsend", msg, nil))
			add("unsend_within_window", observed(r, err))
		case "MSG-MARK-READ":
			for _, op := range []string{"mark_unread", "mark_read"} {
				r, err = submit(target(op, chat, nil))
				add(op, observed(r, err))
			}
		case "MSG-DELETE":
			r, err = submit(target("delete_message", msg, nil))
			add("delete_message_complete_scan", observed(r, err))
			chat = record("messages.chat", chat.ID)
			r, err = submit(target("delete_chat", chat, nil))
			add("delete_chat_complete_scan", observed(r, err))
		case "MSG-UI-INTERFERENCE":
			for _, name := range []string{"ui-interference-before", "ui-interference"} {
				fault(name)
				c := target("reply", msg, map[string]any{"text": text})
				r, err = submit(c)
				if name == "ui-interference-before" {
					add(name, err != nil && r.State == contracts.CommandRejected && count(c, 0))
				} else {
					add(name, err == nil && r.State == contracts.CommandIndeterminate && count(c, 0))
				}
			}
		}
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
