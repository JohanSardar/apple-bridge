package scenarios

import (
	"context"
	"encoding/json"
	"net/http"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
)

func runRelayAgents(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("relay-agents"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	w, err := world.New(world.WithGatewayDials(ctx), "sim", 1, 3, world.BlobOptions{Scope: "reminders:write,system:read"})
	add("paired_replica_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	defer func() {
		e.Passed = true
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	gateway, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("gateway_authority", false)
		return
	}
	replica, err := trust.Open(w.Replicas[0].Home)
	if err != nil {
		add("replica_authority", false)
		return
	}
	session, _, err := gateway.CreateDeviceSession("trusted")
	if err != nil {
		add("owner_session", false)
		return
	}
	owner := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: session}}
	var lists contracts.ReadResult
	_, err = world.Request(ctx, w.Replicas[0], "/v1/op/records.query", contracts.ReadQuery{Collection: "reminders.list"}, &lists)
	if err != nil || len(lists.Records) != 1 {
		add("bridge_test_list", false)
		return
	}
	command := func() contracts.Command {
		args, _ := json.Marshal(map[string]any{"list_id": lists.Records[0].ID, "title": "[ab-test " + e.RunID + "] delegated agent"})
		return contracts.Command{ID: contracts.NewCommandID(), Op: "reminders.create", Args: args, WaitMS: 15000}
	}
	call := func(token string, c contracts.Command) (int, contracts.CommandRecord) {
		endpoint := w.Replicas[0]
		endpoint.Token = token
		var out contracts.CommandRecord
		status, _ := world.Request(ctx, endpoint, "/v1/op/reminders.create", c, &out)
		return status, out
	}
	create := func(level trust.AgentLevel) (trust.Agent, string) {
		agent, token, err := replica.CreateAgent("Bridge Test replica "+string(level), level, []contracts.Grant{"reminders:write"})
		add("create_replica_"+string(level), err == nil)
		return agent, token
	}
	_, read := create(trust.AgentRead)
	denied := command()
	status, _ := call(read, denied)
	effects, _ := w.Effects()
	add("replica_read_agent_denied", status == 401 && effects[denied.ID] == 0)
	fullAgent, full := create(trust.AgentFull)
	relayAgentDelegationBoundaries(ctx, w, replica, fullAgent.ID, command, add)
	immediate := command()
	status, record := call(full, immediate)
	add("replica_full_agent_forwarded", status == 200 && record.State == contracts.CommandObserved)
	_, approve := create(trust.AgentApprove)
	connection, err := owner.socket()
	add("gateway_owner_live_connection", err == nil)
	if err != nil {
		return
	}
	defer connection.CloseNow()
	pending := command()
	status, record = call(approve, pending)
	add("replica_approve_agent_queued_on_gateway", status == 200 && record.State == contracts.CommandAwaitingApproval)
	if status != 200 || record.State != contracts.CommandAwaitingApproval {
		return
	}
	liveContext, stop := context.WithTimeout(ctx, 3*time.Second)
	var event struct {
		T       string `json:"t"`
		Command struct {
			ID string `json:"command_id"`
		} `json:"command"`
	}
	err = wsjson.Read(liveContext, connection, &event)
	stop()
	add("delegated_agent_pending_approval_live_event", err == nil && event.T == "approval" && event.Command.ID == pending.ID)
	effects, _ = w.Effects()
	add("no_effect_before_gateway_owner_consent", effects[pending.ID] == 0)
	response, _, err := owner.request("POST", "approvals/"+pending.ID, map[string]string{"decision": "approve"})
	add("gateway_owner_approves_replica_agent", err == nil && response.StatusCode == 200)
	endpoint := w.Replicas[0]
	endpoint.Token = approve
	err = w.Await(5*time.Second, func() bool {
		var row contracts.CommandRecord
		_, err := world.Request(ctx, endpoint, "/v1/op/commands.status", contracts.CommandQuery{ID: pending.ID}, &row)
		return err == nil && row.State == contracts.CommandObserved
	})
	effects, _ = w.Effects()
	add("gateway_approval_executes_once", err == nil && effects[pending.ID] == 1)
	relayAgentApprovalBoundaries(ctx, w, owner, replica, create, call, command, add)
	return
}
