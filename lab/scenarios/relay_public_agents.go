package scenarios

import (
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

func relayPublicAgentChecks(ctx context.Context, w *world.World, e *Evidence, add func(string, bool), enrollment trust.Enrollment) {
	prefix := "[ab-test " + contracts.NewID("relay-agent") + "]"
	if w.Native {
		_, err := w.Truth("fixture", map[string]any{"action": "create_list", "list": "Bridge Test", "tag": prefix})
		add("native_agent_fixture_list", err == nil)
		if err != nil {
			return
		}
		defer func() {
			_, err := w.RecoverTruth("fixture", map[string]any{"action": "cleanup", "list": "Bridge Test", "tag": prefix})
			add("native_agent_tagged_cleanup", err == nil)
		}()
	}
	authority, err := trust.Open(w.Gateway.Home)
	if err != nil {
		add("relay_agent_gateway_authority", false)
		return
	}
	session, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		add("relay_agent_owner_session", false)
		return
	}
	owner := &webClient{ctx: ctx, origin: strings.Replace(w.Gateway.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: session}}
	connection, err := owner.socket()
	add("relay_agent_owner_live_connection", err == nil)
	if err != nil {
		return
	}
	defer connection.CloseNow()
	fullID, approveID := contracts.NewCommandID(), contracts.NewCommandID()
	input := relayProbeInput(w)
	input["Prefix"], input["FullID"], input["ApproveID"] = prefix, fullID, approveID
	var result struct {
		Read          bool `json:"read_denied"`
		Full          bool `json:"full_observed"`
		Pending       bool `json:"approve_queued"`
		Approved      bool `json:"approved_observed"`
		ReadYourWrite bool `json:"read_your_write"`
		Cleanup       bool `json:"cleanup_observed"`
	}
	err = w.RelayExchange("agents", input, func() (any, error) {
		bounded, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		var event struct {
			T       string `json:"t"`
			Command struct {
				ID string `json:"command_id"`
			} `json:"command"`
		}
		var err error
		for {
			err = wsjson.Read(bounded, connection, &event)
			if err != nil || event.T == "approval" && event.Command.ID == approveID {
				break
			}
		}
		add("public_delegated_approval_live_event", err == nil && event.T == "approval" && event.Command.ID == approveID)
		if err != nil {
			return nil, err
		}
		response, raw, err := owner.request("GET", "approvals", nil)
		var pending struct {
			Approvals []struct {
				ID        string `json:"command_id"`
				Principal string `json:"principal"`
			} `json:"approvals"`
		}
		composite := false
		if err == nil && response.StatusCode == 200 && json.Unmarshal(raw, &pending) == nil {
			for _, row := range pending.Approvals {
				composite = composite || row.ID == approveID && strings.HasPrefix(row.Principal, enrollment.ReplicaID+"/agent:")
			}
		}
		add("public_pending_approval_bound_to_paired_replica_agent", composite)
		read, readErr := relayAgentVisible(w, prefix, " read")
		add("independent_public_read_agent_has_no_effect", readErr == nil && !read)
		full, fullErr := relayAgentVisible(w, prefix, " full")
		waiting, waitingErr := relayAgentVisible(w, prefix, " approve")
		add("independent_full_effect_and_no_effect_before_consent", fullErr == nil && waitingErr == nil && full && !waiting)
		if !composite || readErr != nil || read || fullErr != nil || waitingErr != nil || !full || waiting {
			return nil, fmt.Errorf("relay agent independent pre-approval check failed")
		}
		response, _, err = owner.request("POST", "approvals/"+approveID, map[string]string{"decision": "approve"})
		add("gateway_owner_approves_public_replica_agent", err == nil && response.StatusCode == 200)
		if err != nil || response.StatusCode != 200 {
			return nil, fmt.Errorf("public agent owner approval failed")
		}
		err = w.Await(15*time.Second, func() bool { visible, err := relayAgentVisible(w, prefix, " approve"); return err == nil && visible })
		add("independent_approved_effect", err == nil)
		return map[string]bool{"consent_verified": err == nil}, err
	}, &result)
	add("public_replica_agent_levels_and_command_read_your_write", err == nil && result.Read && result.Full && result.Pending && result.Approved && result.ReadYourWrite && result.Cleanup)
	if err != nil {
		return
	}
	full, fullErr := relayAgentVisible(w, prefix, " full")
	approved, approvedErr := relayAgentVisible(w, prefix, " approve")
	read, readErr := relayAgentVisible(w, prefix, " read")
	add("independent_public_agent_delete_absence", fullErr == nil && approvedErr == nil && readErr == nil && !read && !full && !approved)
	if !w.Native {
		effects, err := w.Effects()
		once := err == nil && effects[fullID] == 1 && effects[approveID] == 1
		add("public_agent_creates_execute_once", once)
	}
}
func relayAgentVisible(w *world.World, prefix, suffix string) (bool, error) {
	if w.Native {
		truth, err := w.Truth("observe", map[string]any{"list": "Bridge Test", "tag": prefix, "title": prefix + suffix})
		records, _ := truth["records"].([]any)
		return len(records) == 1, err
	}
	records, err := w.Inspect()
	for _, record := range records {
		var value struct {
			Title string `json:"title"`
		}
		if record.Kind == "upsert" && json.Unmarshal(record.Data, &value) == nil && value.Title == prefix+suffix {
			return true, err
		}
	}
	return false, err
}
