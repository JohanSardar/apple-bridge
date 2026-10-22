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
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
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
func captureObserverClient(ctx context.Context, endpoint world.Endpoint) (*webClient, error) {
	authority, err := trust.Open(endpoint.Home)
	if err != nil {
		return nil, err
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		return nil, err
	}
	return &webClient{ctx: ctx, origin: strings.Replace(endpoint.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}, nil
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
	client, err := captureObserverClient(ctx, w.Replicas[0])
	add("replica_web_session", err == nil)
	if err != nil {
		return
	}
	socket, err := client.socket()
	add("replica_web_socket", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	readCtx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	err = wsjson.Write(readCtx, socket, map[string]any{"t": "sub", "id": "capture", "view": "record", "args": map[string]any{"collection": target.Collection, "id": target.ID}})
	var frame map[string]any
	if err == nil {
		err = wsjson.Read(readCtx, socket, &frame)
	}
	add("record_snapshot_before_capture", err == nil && frame["t"] == "snap")
	if err != nil {
		return
	}
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
	frame = nil
	err = wsjson.Read(readCtx, socket, &frame)
	add("web_view_reset_before_any_capture_delta", err == nil && frame["t"] == "error" && frame["code"] == "reset")
	if err != nil {
		return
	}
	wsjson.Write(readCtx, socket, map[string]any{"t": "unsub", "id": "capture"})
	wsjson.Write(readCtx, socket, map[string]any{"t": "sub", "id": "capture", "view": "record", "args": map[string]any{"collection": target.Collection, "id": target.ID}})
	frame = nil
	err = wsjson.Read(readCtx, socket, &frame)
	var snap struct {
		Records []contracts.Record `json:"records"`
		Cursor  contracts.Cursor   `json:"cursor"`
	}
	raw, _ := json.Marshal(frame["data"])
	json.Unmarshal(raw, &snap)
	add("web_view_resnapshot_has_committed_capture", err == nil && frame["t"] == "snap" && len(snap.Records) == 1 && recordTitle(snap.Records[0]) == "[ab-test capture observer] "+prefix+"-0" && snap.Cursor.Sequence >= through.Sequence)
}
func captureApprovalObserver(ctx context.Context, w *world.World, owner *webClient, add func(string, bool)) {
	response, raw, err := owner.request("POST", "agents", map[string]any{"name": "Bridge Test capture approval", "level": "approve", "scopes": []string{"messages:write"}})
	var agent struct {
		Token string `json:"token"`
	}
	json.Unmarshal(raw, &agent)
	add("approval_agent_created", err == nil && response.StatusCode == 201 && agent.Token != "")
	if err != nil || agent.Token == "" {
		return
	}
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: mustJSON(map[string]any{"handles": []string{"self@example.com"}, "service": "iMessage", "text": "[ab-test capture observer] pending approval"})}
	code, raw, _ := agentRequest(ctx, owner.origin, agent.Token, "POST", "/v1/op/messages.send", command)
	var pending contracts.CommandRecord
	json.Unmarshal(raw, &pending)
	add("approval_pending_before_observation", code == 200 && pending.State == contracts.CommandAwaitingApproval)
	if code != 200 || pending.State != contracts.CommandAwaitingApproval {
		return
	}
	defer owner.request("POST", "approvals/"+command.ID, map[string]string{"decision": "reject"})
	socket, err := owner.socket()
	add("approval_socket_registered_after_pending", err == nil)
	if err != nil {
		return
	}
	defer socket.CloseNow()
	err = w.Await(10*time.Second, func() bool { return convergence(w) })
	add("approval_pending_replicated_before_observation", err == nil)
	replica, replicaErr := captureObserverClient(ctx, w.Replicas[0])
	if replicaErr != nil {
		add("replica_approval_socket_registered_after_pending", false)
		return
	}
	replicaSocket, replicaErr := replica.socket()
	add("replica_approval_socket_registered_after_pending", replicaErr == nil)
	if replicaErr != nil {
		return
	}
	defer replicaSocket.CloseNow()
	_, err = captureObserverChanges(w, contracts.Reminders, "reminders.reminder", "capture-approval", 150)
	add("approval_oversized_capture_committed", err == nil)
	if err != nil {
		return
	}
	timed, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	var frame struct {
		T       string `json:"t"`
		Command struct {
			ID string `json:"command_id"`
		} `json:"command"`
	}
	err = wsjson.Read(timed, socket, &frame)
	add("approval_watch_rereads_pending_after_reset", err == nil && frame.T == "approval" && frame.Command.ID == command.ID)
	replicaTimed, replicaCancel := context.WithTimeout(ctx, 5*time.Second)
	defer replicaCancel()
	frame.T, frame.Command.ID = "", ""
	err = wsjson.Read(replicaTimed, replicaSocket, &frame)
	add("replica_approval_watch_rereads_pending_after_reset", err == nil && frame.T == "approval" && frame.Command.ID == command.ID)
	effects, effectErr := w.Effects()
	add("approval_reset_never_executes_pending_command", effectErr == nil && effects[command.ID] == 0)
	response, _, err = owner.request("POST", "approvals/"+command.ID, map[string]string{"decision": "approve"})
	add("approval_resolved_on_gateway", err == nil && response.StatusCode == 200)
	if err != nil || response.StatusCode != 200 {
		return
	}
	err = w.Await(10*time.Second, func() bool { return convergence(w) })
	add("approval_resolution_replicated", err == nil)
	_, err = captureObserverChanges(w, contracts.Reminders, "reminders.reminder", "capture-resolved-approval", 150)
	add("resolved_approval_oversized_capture_committed", err == nil)
	if err != nil {
		return
	}
	err = w.Await(10*time.Second, func() bool { return convergence(w) })
	add("resolved_approval_capture_replicated", err == nil)
	noPrompt, noPromptCancel := context.WithTimeout(ctx, time.Second)
	defer noPromptCancel()
	frame.T, frame.Command.ID = "", ""
	err = wsjson.Read(noPrompt, replicaSocket, &frame)
	add("replica_resolved_approval_never_reemitted_after_reset", err != nil && noPrompt.Err() != nil)

}
