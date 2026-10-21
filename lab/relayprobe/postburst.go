package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type postBurstResult struct {
	InitialHTTP int                    `json:"initial_http_status"`
	HTTP        int                    `json:"http_status"`
	Code        string                 `json:"error_code"`
	State       contracts.CommandState `json:"state"`
	InitialMS   float64                `json:"initial_response_ms"`
	ElapsedMS   float64                `json:"elapsed_ms"`
	TTLMS       int64                  `json:"normal_ttl_ms"`
	Observed    bool                   `json:"observed"`
	LocalAbsent bool                   `json:"local_absent"`
	CommandID   string                 `json:"command_id"`
}

func postburst(ctx context.Context) error {
	decoder := json.NewDecoder(os.Stdin)
	var in struct {
		input
		Title, DeleteID string
	}
	if err := decoder.Decode(&in); err != nil {
		return err
	}
	authority, err := trust.Open(in.Home)
	if err != nil {
		return err
	}
	_, token, err := authority.CreateAgent("Bridge Test post-burst full", trust.AgentFull, []contracts.Grant{"reminders:write"})
	if err != nil {
		return err
	}
	local := in.input
	local.Token = token
	lists, err := named(ctx, local, "reminders.list", "Bridge Test")
	if err != nil || len(lists) != 1 {
		return fmt.Errorf("post-burst list unavailable")
	}
	args, _ := json.Marshal(map[string]string{"list_id": lists[0].ID, "title": in.Title})
	var created contracts.CommandRecord
	if err = request(ctx, local, "reminders.create", contracts.Command{ID: contracts.NewCommandID(), Op: "reminders.create", Args: args, WaitMS: 15000}, &created); err != nil {
		return err
	}
	records, err := named(ctx, local, "reminders.reminder", in.Title)
	if err != nil || created.State != contracts.CommandObserved || len(records) != 1 {
		return fmt.Errorf("post-burst fixture not observed")
	}
	target := records[0]
	if err = json.NewEncoder(os.Stdout).Encode(map[string]bool{"ready": true}); err != nil {
		return err
	}
	var signal any
	if err = decoder.Decode(&signal); err != nil {
		return err
	}
	records, err = named(ctx, local, "reminders.reminder", in.Title)
	if err != nil || len(records) != 1 {
		return fmt.Errorf("post-burst fixture unavailable before delete")
	}
	target = records[0]
	ttl, _ := contracts.CommandTTL("reminders.delete")
	bounded, cancel := context.WithTimeout(ctx, ttl)
	defer cancel()
	args, _ = json.Marshal(map[string]string{"id": target.ID})
	command := contracts.Command{ID: in.DeleteID, Op: "reminders.delete", Args: args, ExpectedVersion: target.Version, WaitMS: 15000}
	result := postBurstResult{CommandID: in.DeleteID, TTLMS: ttl.Milliseconds()}
	var record contracts.CommandRecord
	started := time.Now()
	err = request(bounded, local, "reminders.delete", command, &record)
	result.InitialMS = float64(time.Since(started)) / float64(time.Millisecond)
	result.InitialHTTP = 200
	if err != nil {
		result.InitialHTTP = 0
		var problem *localAPIError
		if errors.As(err, &problem) {
			result.InitialHTTP = problem.Status
		}
	}
	for err == nil && record.State != contracts.CommandObserved && (record.State == contracts.CommandAdmitted || record.State == contracts.CommandPrepared || record.State == contracts.CommandDispatched) {
		select {
		case <-bounded.Done():
			err = bounded.Err()
		case <-time.After(time.Second):
			err = request(bounded, local, "commands.status", contracts.CommandQuery{ID: in.DeleteID}, &record)
		}
	}
	result.ElapsedMS = float64(time.Since(started)) / float64(time.Millisecond)
	result.HTTP = 200
	result.State = record.State
	if err != nil {
		result.HTTP = 0
		result.Code = "transport_error"
		var problem *localAPIError
		if errors.As(err, &problem) {
			result.HTTP = problem.Status
			result.Code = problem.Code
		}
	} else {
		result.Observed = record.State == contracts.CommandObserved && time.Since(started) <= ttl
		if record.Error != nil {
			result.Code = record.Error.Code
		}
		if result.Observed {
			records, err = named(bounded, local, "reminders.reminder", in.Title)
			result.LocalAbsent = err == nil && len(records) == 0
		}
	}
	return json.NewEncoder(os.Stdout).Encode(result)
}
