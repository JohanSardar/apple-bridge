package scenarios

import (
	"context"
	"encoding/json"
	"net/http"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"github.com/coder/websocket/wsjson"
)

type sourceProof struct {
	Source       string             `json:"source"`
	State        string             `json:"state"`
	Coverage     contracts.Coverage `json:"coverage"`
	Capabilities []string           `json:"capabilities"`
}

func messageReactionProof(records []contracts.Record) (contracts.Record, sourceProof, contracts.OperationCapability) {
	for _, record := range records {
		var source sourceProof
		if json.Unmarshal(record.Data, &source) == nil && source.Source == "messages" {
			for _, proof := range source.Coverage.Operations {
				if proof.Operation == "messages.react" {
					return record, source, proof
				}
			}
		}
	}
	return contracts.Record{}, sourceProof{}, contracts.OperationCapability{}
}

func runWebSources(ctx context.Context, w *world.World, client *webClient, auth *webAuthenticator, evidence *Evidence, add func(string, bool)) {
	unauthorized := *client
	unauthorized.cookie = nil
	response, _, err := unauthorized.request("GET", "sources", nil)
	add("source_status_requires_device_session", err == nil && response.StatusCode == 401)
	response, body, err := client.request("GET", "sources", nil)
	add("source_status_device_read", err == nil && response.StatusCode == 200)
	if err != nil || response.StatusCode != 200 {
		return
	}
	var result contracts.ReadResult
	err = json.Unmarshal(body, &result)
	add("source_state_ids_and_coverage", err == nil && len(result.Records) >= 6 && response.Header.Get("Cache-Control") == "no-store")
	record, source, proof := messageReactionProof(result.Records)
	add("source_proofs_have_reasons", record.ID != "" && record.Collection == "system.source" && source.State == "ready" && proof.Reason != "")
	conn, err := client.socket()
	if err != nil {
		add("source_live_socket", false)
		return
	}
	defer conn.CloseNow()
	err = wsjson.Write(ctx, conn, map[string]any{"t": "sub", "id": "source-proof", "view": "record", "args": map[string]any{"collection": "system.source", "id": record.ID}})
	add("source_id_subscribed", err == nil)
	for _, proven := range []bool{false, true} {
		action := "capability-unproven"
		if proven {
			action = "capability-proven"
		}
		started := time.Now()
		add(action+"_published", w.ControlAction("/messages", map[string]any{"action": action}) == nil)
		bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
		matched := false
		for !matched {
			var frame struct {
				Type    string               `json:"t"`
				Data    contracts.ReadResult `json:"data"`
				Changes []contracts.Change   `json:"changes"`
			}
			if wsjson.Read(bounded, conn, &frame) != nil {
				break
			}
			records := frame.Data.Records
			for _, change := range frame.Changes {
				records = append(records, change.Record)
			}
			current, state, operation := messageReactionProof(records)
			matched = current.ID == record.ID && operation.Proven == proven && operation.Reason != "" && slices.Contains(state.Capabilities, "messages.react") == proven
			if matched && proven {
				add("proof_appears_as_live_delta", frame.Type == "delta")
			}
		}
		cancel()
		add(action+"_live_action_advertisement", matched)
		evidence.Latency[action+"_to_live_source"] = stats([]float64{float64(time.Since(started)) / float64(time.Millisecond)}, 0)
	}
	status, body, err := client.signIn(auth, "login", "", "public")
	var session struct {
		Kind  string `json:"kind"`
		Cache string `json:"cache"`
	}
	json.Unmarshal(body, &session)
	add("public_source_session", err == nil && status == 200 && session.Kind == "public" && session.Cache == "memory")
	response, body, err = client.request("GET", "sources", nil)
	var public contracts.ReadResult
	json.Unmarshal(body, &public)
	publicRecord, _, publicProof := messageReactionProof(public.Records)
	add("public_sees_same_proof_without_cache", err == nil && response.StatusCode == 200 && response.Header.Get("Cache-Control") == "no-store" && publicRecord.ID == record.ID && publicProof.Proven)
	var expected contracts.ReadResult
	err = (scripted.Client{Endpoint: w.Gateway}).Call(ctx, "sources.status", struct{}{}, &expected)
	add("source_route_matches_operations", err == nil && reflect.DeepEqual(expected.Records, public.Records))
	add("scoped_replica_started", w.AddReplicaScope("reminders:read,system:read") == nil)
	if len(w.Replicas) == 0 {
		return
	}
	endpoint := w.Replicas[0]
	authority, err := trust.Open(endpoint.Home)
	if err != nil {
		add("scoped_source_authority", false)
		return
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	if err != nil {
		add("scoped_source_device", false)
		return
	}
	_, port, _ := strings.Cut(endpoint.Host, ":")
	scoped := webClient{ctx: ctx, origin: "http://localhost:" + port, cookie: &http.Cookie{Name: "ab_session_local", Value: token}}
	add("scoped_source_visibility", w.Await(5*time.Second, func() bool {
		response, body, err := scoped.request("GET", "sources", nil)
		var sources contracts.ReadResult
		if err != nil || response.StatusCode != 200 || json.Unmarshal(body, &sources) != nil || len(sources.Records) == 0 {
			return false
		}
		for _, record := range sources.Records {
			if record.Source != contracts.Reminders && record.Source != contracts.System {
				return false
			}
		}
		return true
	}) == nil)
}
