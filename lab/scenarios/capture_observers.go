package scenarios

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type captureObserverResult struct {
	reset bool
	batch bool
}

func captureObserverSSE(ctx context.Context, endpoint world.Endpoint, after contracts.Cursor) (<-chan captureObserverResult, error) {
	request, _ := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/changes?after="+after.EventID(), nil)
	request.Host = endpoint.Host
	request.Header.Set("Authorization", "Bearer "+endpoint.Token)
	response, err := http.DefaultClient.Do(request)
	if err != nil {
		return nil, err
	}
	if response.StatusCode != 200 {
		response.Body.Close()
		return nil, fmt.Errorf("changes status %d", response.StatusCode)
	}
	done := make(chan captureObserverResult, 1)
	go func() {
		defer response.Body.Close()
		defer close(done)
		scanner := bufio.NewScanner(response.Body)
		scanner.Buffer(make([]byte, 4096), contracts.MaxFrame*2)
		event := ""
		for scanner.Scan() {
			line := scanner.Text()
			if strings.HasPrefix(line, "event: ") {
				event = strings.TrimPrefix(line, "event: ")
				continue
			}
			if !strings.HasPrefix(line, "data: ") {
				continue
			}
			if event == "reset" {
				var body struct {
					Error string `json:"error"`
				}
				json.Unmarshal([]byte(strings.TrimPrefix(line, "data: ")), &body)
				done <- captureObserverResult{reset: body.Error == "reset"}
			} else {
				done <- captureObserverResult{batch: event == "batch"}
			}
			return
		}
	}()
	return done, nil
}
func runCaptureObservers(ctx context.Context, w *world.World, owner *webClient, evidence *Evidence, add func(string, bool)) {
	captureApprovalObserver(ctx, w, owner, add)
	prefix := "capture-observer"
	err := w.ControlAction("/external-change", map[string]any{"key": prefix + "-0", "collection": "reminders.reminder", "data": map[string]any{"title": "[ab-test capture observer] before"}})
	if err == nil {
		err = w.FlushCaptures(5 * time.Second)
	}
	if err == nil {
		err = w.Await(10*time.Second, func() bool { return convergence(w) })
	}
	add("observer_fixture_converges", err == nil)
	if err != nil {
		return
	}
	var target contracts.Record
	records, _ := w.Records(w.Replicas[0])
	for _, record := range records {
		if recordTitle(record) == "[ab-test capture observer] before" {
			target = record
			break
		}
	}
	add("observer_target", target.ID != "")
	if target.ID == "" {
		return
	}
	readCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	observer, err := watch(ctx, w.Replicas[0], false, func(contracts.Record) bool { return true })
	add("ordinary_sse_client_registered", err == nil)
	if err != nil {
		return
	}
	defer observer.close()
	before, _ := w.Cursor(w.Replicas[0])
	result, err := captureObserverSSE(readCtx, w.Replicas[0], before)
	add("rest_observer_registered", err == nil)
	if err != nil {
		return
	}
	through, err := captureObserverChanges(w, contracts.Reminders, "reminders.reminder", prefix, 150)
	add("oversized_observer_capture_committed", err == nil)
	if err != nil {
		return
	}
	err = w.Await(10*time.Second, func() bool {
		local, e := w.Cursor(w.Replicas[0])
		return e == nil && local.Epoch == through.Epoch && local.Sequence >= through.Sequence
	})
	add("oversized_observer_capture_applied", err == nil)
	observed := w.Await(5*time.Second, func() bool {
		observer.mu.Lock()
		defer observer.mu.Unlock()
		return observer.last.Sequence >= through.Sequence
	}) == nil
	_, present := observer.observedExact("[ab-test capture observer] " + prefix + "-0")
	observer.mu.Lock()
	valid := !observer.invalid && observer.snapshotResets > 0
	observer.mu.Unlock()
	add("ordinary_sse_client_resnapshots_without_partial_rows", observed && present && valid)
	select {
	case seen := <-result:
		add("rest_reset_before_any_capture_batch", seen.reset && !seen.batch)
	case <-readCtx.Done():
		add("rest_reset_before_any_capture_batch", false)
	}
	var snapshot contracts.ReadResult
	_, err = world.Request(ctx, w.Replicas[0], "/v1/op/records.query", contracts.ReadQuery{Collection: target.Collection}, &snapshot)
	found := false
	for _, row := range snapshot.Records {
		found = found || row.ID == target.ID && recordTitle(row) == "[ab-test capture observer] "+prefix+"-0"
	}
	add("rest_resnapshot_has_committed_capture", err == nil && found && snapshot.Cursor.Sequence >= through.Sequence)
}

func captureApprovalObserver(ctx context.Context, w *world.World, owner *webClient, add func(string, bool)) {
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("approval_authority", false)
		return
	}
	_, token, err := authority.CreateAgent("Bridge Test capture approval", trust.AgentApprove, []contracts.Grant{"messages:write"})
	add("approval_agent_created", err == nil)
	if err != nil {
		return
	}
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: mustJSON(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "[ab-test capture observer] pending approval"})}
	code, raw, _ := agentRequest(ctx, owner.origin, token, "POST", "/v1/op/messages.send", command)
	var pending contracts.CommandRecord
	json.Unmarshal(raw, &pending)
	add("approval_pending_before_observation", code == 200 && pending.State == contracts.CommandAwaitingApproval)
	if code != 200 || pending.State != contracts.CommandAwaitingApproval {
		return
	}
	_, err = captureObserverChanges(w, contracts.Reminders, "reminders.reminder", "capture-approval", 150)
	add("approval_oversized_capture_committed", err == nil)
	err = w.Await(10*time.Second, func() bool { return convergence(w) })
	add("approval_pending_replicated_after_capture", err == nil)
	replicaApproval := func(pending bool) bool {
		var records contracts.ReadResult
		_, err := world.Request(ctx, w.Replicas[0], "/v1/op/records.query", contracts.ReadQuery{Collection: "system.command"}, &records)
		if err != nil {
			return false
		}
		for _, row := range records.Records {
			if row.ID == command.ID {
				var data struct {
					Pending bool `json:"awaiting_approval"`
				}
				return json.Unmarshal(row.Data, &data) == nil && data.Pending == pending
			}
		}
		return false
	}
	add("replica_pending_approval_survives_capture", replicaApproval(true))
	response, body, err := owner.request("GET", "approvals", nil)
	add("consent_page_rereads_pending_after_capture", err == nil && response.StatusCode == 200 && strings.Contains(string(body), command.ID))
	effects, effectErr := w.Effects()
	add("approval_capture_never_executes_pending_command", effectErr == nil && effects[command.ID] == 0)
	response, _, err = owner.request("POST", "approvals/"+command.ID, map[string]string{"decision": "approve"})
	add("approval_resolved_on_gateway", err == nil && response.StatusCode == 200)
	_, err = captureObserverChanges(w, contracts.Reminders, "reminders.reminder", "capture-resolved-approval", 150)
	add("resolved_approval_oversized_capture_committed", err == nil)
	err = w.Await(10*time.Second, func() bool { return convergence(w) })
	add("resolved_approval_capture_replicated", err == nil)
	add("replica_resolved_approval_survives_capture", replicaApproval(false))
	response, body, err = owner.request("GET", "approvals", nil)
	add("resolved_approval_absent_from_consent_page", err == nil && response.StatusCode == 200 && !strings.Contains(string(body), command.ID))
}
