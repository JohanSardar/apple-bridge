package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func agentReadDenied(ctx context.Context, in input, command contracts.Command) bool {
	raw, _ := json.Marshal(command)
	req, err := http.NewRequestWithContext(ctx, "POST", in.URL+"/v1/op/reminders.create", bytes.NewReader(raw))
	if err != nil {
		return false
	}
	req.Host = in.Host
	req.Header.Set("Authorization", "Bearer "+in.Token)
	req.Header.Set("Content-Type", "application/json")
	response, err := http.DefaultClient.Do(req)
	if err != nil {
		return false
	}
	response.Body.Close()
	return response.StatusCode == 401
}
func agents(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 75*time.Second)
	defer cancel()
	decoder := json.NewDecoder(os.Stdin)
	var in struct {
		input
		Prefix, FullID, ApproveID string
	}
	if err := decoder.Decode(&in); err != nil {
		return err
	}
	authority, err := trust.Open(in.Home)
	if err != nil {
		return err
	}
	tokens := map[trust.AgentLevel]string{}
	for _, level := range []trust.AgentLevel{trust.AgentRead, trust.AgentFull, trust.AgentApprove} {
		_, token, err := authority.CreateAgent("Bridge Test relay "+string(level), level, []contracts.Grant{"reminders:write"})
		if err != nil {
			return err
		}
		tokens[level] = token
	}
	var lists []contracts.Record
	deadline := time.Now().Add(10 * time.Second)
	for {
		lists, err = named(ctx, in.input, "reminders.list", "Bridge Test")
		if err == nil && len(lists) == 1 {
			break
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("Bridge Test list unavailable")
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(20 * time.Millisecond):
		}
	}
	create := func(id, suffix string) contracts.Command {
		args, _ := json.Marshal(map[string]string{"list_id": lists[0].ID, "title": in.Prefix + suffix})
		return contracts.Command{ID: id, Op: "reminders.create", Args: args, WaitMS: 15000}
	}
	local := in.input
	local.Token = tokens[trust.AgentRead]
	if !agentReadDenied(ctx, local, create(contracts.NewCommandID(), " read")) {
		return fmt.Errorf("read agent was not denied")
	}
	local.Token = tokens[trust.AgentFull]
	var full contracts.CommandRecord
	if err := request(ctx, local, "reminders.create", create(in.FullID, " full"), &full); err != nil {
		return err
	}
	records, err := named(ctx, local, "reminders.reminder", in.Prefix+" full")
	if err != nil || full.State != contracts.CommandObserved || len(records) != 1 {
		return fmt.Errorf("full agent create not observed with read-your-write")
	}
	local.Token = tokens[trust.AgentApprove]
	var pending contracts.CommandRecord
	if err := request(ctx, local, "reminders.create", create(in.ApproveID, " approve"), &pending); err != nil {
		return err
	}
	records, err = named(ctx, local, "reminders.reminder", in.Prefix+" approve")
	if err != nil || pending.State != contracts.CommandAwaitingApproval || len(records) != 0 {
		return fmt.Errorf("approve agent did not wait without an effect")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]bool{"ready": true}); err != nil {
		return err
	}
	var consent any
	if err := decoder.Decode(&consent); err != nil {
		return err
	}
	for {
		if err := request(ctx, local, "commands.status", contracts.CommandQuery{ID: in.ApproveID}, &pending); err != nil {
			return err
		}
		if pending.State == contracts.CommandObserved {
			break
		}
		if pending.State != contracts.CommandAdmitted && pending.State != contracts.CommandPrepared && pending.State != contracts.CommandDispatched && pending.State != contracts.CommandAwaitingApproval {
			return fmt.Errorf("approved command did not complete")
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(20 * time.Millisecond):
		}
	}
	records, err = named(ctx, local, "reminders.reminder", in.Prefix+" approve")
	if err != nil || len(records) != 1 {
		return fmt.Errorf("approved agent read-your-write missing")
	}
	local.Token = tokens[trust.AgentFull]
	for _, suffix := range []string{" full", " approve"} {
		records, err := named(ctx, local, "reminders.reminder", in.Prefix+suffix)
		if err != nil || len(records) != 1 {
			return fmt.Errorf("agent cleanup fixture unavailable")
		}
		args, _ := json.Marshal(map[string]string{"id": records[0].ID})
		remove := contracts.Command{ID: contracts.NewCommandID(), Op: "reminders.delete", Args: args, ExpectedVersion: records[0].Version, WaitMS: 15000}
		var deleted contracts.CommandRecord
		if err = request(ctx, local, "reminders.delete", remove, &deleted); err != nil {
			return err
		}
		records, err = named(ctx, local, "reminders.reminder", in.Prefix+suffix)
		if err != nil || deleted.State != contracts.CommandObserved || len(records) != 0 {
			return fmt.Errorf("agent cleanup not observed")
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]bool{"read_denied": true, "full_observed": true, "approve_queued": true, "approved_observed": true, "read_your_write": true, "cleanup_observed": true})
}
