package scenarios

import (
	"context"
	"database/sql"
	"path/filepath"
	"reflect"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func rmExactParticipants(ctx context.Context, w *world.World, add func(string, bool)) {
	addresses := []string{"participant.a@example.com", "participant.b@example.com", "participant.c@example.com"}
	records := []contracts.Observation{}
	for index, key := range []string{"a", "b", "c"} {
		records = append(records, rmObservation("messages.handle", "rm-exact-handle-"+key, map[string]any{"address": addresses[index], "service": "iMessage"}))
	}
	chat := func(key string, members ...string) contracts.Observation {
		participants := []any{}
		for _, member := range members {
			participants = append(participants, rmRef("messages.handle", "rm-exact-handle-"+member))
		}
		style := "direct"
		if len(members) > 1 {
			style = "group"
		}
		return rmObservation("messages.chat", "rm-exact-chat-"+key, map[string]any{"display_name": "Bridge Test exact " + key, "participants": participants, "style": style, "service": "iMessage"})
	}
	records = append(records, chat("a", "a"), chat("ab", "a", "b"), chat("abc", "a", "b", "c"))
	if err := rmPut(w, records...); err != nil {
		add("exact_participant_fixture", false)
		return
	}
	aliases, err := gatewayAliases(w)
	add("exact_participant_aliases", err == nil)
	queries := []struct {
		name    string
		handles []string
		key     string
	}{
		{"single", addresses[:1], "a"},
		{"group", addresses[:2], "ab"},
		{"reordered_set", []string{strings.ToUpper(addresses[1]), addresses[0], addresses[1]}, "ab"},
		{"superset", addresses, "abc"},
		{"different_set", []string{addresses[0], addresses[2]}, ""},
		{"unknown", []string{"participant.unknown@example.com"}, ""},
	}
	for _, query := range queries {
		var prior []string
		for _, protocol := range []string{"rest", "mcp"} {
			client := scripted.Client{Endpoint: w.Gateway, Protocol: protocol}
			var result contracts.ChatsResult
			err := client.Call(ctx, "messages.chats", map[string]any{"participants": query.handles, "limit": 1}, &result)
			ids := []string{}
			for _, row := range result.Chats {
				ids = append(ids, row.Chat.ID)
			}
			want := []string{}
			if query.key != "" {
				want = append(want, aliases[aliasKey(contracts.Messages, "messages.chat", "rm-exact-chat-"+query.key)])
			}
			add(protocol+"_exact_participants_"+query.name, err == nil && reflect.DeepEqual(ids, want) && result.Next == "")
			if protocol == "mcp" {
				add("exact_participants_"+query.name+"_parity", err == nil && reflect.DeepEqual(ids, prior))
			}
			prior = ids
		}
	}
	denied := w.Gateway
	denied.Token, err = w.ScopedToken(w.Gateway, "system:read")
	var hidden contracts.ChatsResult
	err = (scripted.Client{Endpoint: denied}).Call(ctx, "messages.chats", map[string]any{"participants": addresses[:1]}, &hidden)
	add("exact_participants_visibility", err == nil && len(hidden.Chats) == 0)
	check := func(name string, participants []string, key string) {
		for _, protocol := range []string{"rest", "mcp"} {
			var result contracts.ChatsResult
			err := (scripted.Client{Endpoint: w.Gateway, Protocol: protocol}).Call(ctx, "messages.chats", map[string]any{"participants": participants, "limit": 1}, &result)
			valid := err == nil && len(result.Chats) == 0
			if key != "" {
				valid = err == nil && len(result.Chats) == 1 && result.Chats[0].Chat.ID == aliases[aliasKey(contracts.Messages, "messages.chat", "rm-exact-chat-"+key)]
			}
			add(protocol+"_exact_participants_"+name, valid)
		}
	}
	add("exact_participant_membership_change", rmPut(w, chat("ab", "a", "c")) == nil)
	check("old_membership_absent", addresses[:2], "")
	check("new_membership", []string{addresses[0], addresses[2]}, "ab")
	add("exact_participant_address_change", rmPut(w, rmObservation("messages.handle", "rm-exact-handle-a", map[string]any{"address": "participant.changed@example.com", "service": "iMessage"})) == nil)
	check("old_address_absent", addresses[:1], "")
	check("changed_address", []string{"PARTICIPANT.CHANGED@example.com"}, "a")
	for _, invalid := range []any{[]string{}, []string{""}, make([]string, 33)} {
		var result contracts.ChatsResult
		err := (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "messages.chats", map[string]any{"participants": invalid}, &result)
		add("invalid_participant_set_rejected", err != nil)
	}
	w.StopGateway()
	db, err := sql.Open("sqlite", filepath.Join(w.Gateway.Home, "state.db"))
	if err == nil {
		_, err = db.Exec("DROP TABLE chat_participant; DROP TABLE handle_address; DELETE FROM meta WHERE key='chat_participants_version'")
		db.Close()
	}
	add("participant_index_migration_fixture", err == nil)
	add("participant_index_migration_restart", w.RestartGateway() == nil)
	check("rebuilt_address", []string{"participant.changed@example.com"}, "a")
	check("rebuilt_membership", []string{"participant.changed@example.com", addresses[2]}, "ab")

}
