package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func relayContactsChecks(ctx context.Context, w *world.World, e *Evidence, add func(string, bool)) {
	tag := "[ab-test " + e.RunID + "]"
	defer func() { contactsLatencyEvidence(w, e, nil, nil) }()
	fixture := func(action string, fields map[string]any) (map[string]any, error) {
		fields["source"], fields["tag"], fields["action"] = "contacts", tag, action
		return w.Truth("fixture", fields)
	}
	defer func() {
		cleanup, err := w.RecoverTruth("fixture", map[string]any{"source": "contacts", "tag": tag, "action": "cleanup"})
		truth, observed := w.RecoverTruth("observe", map[string]any{"source": "contacts", "tag": tag})
		add("tagged_contacts_cleaned", err == nil && observed == nil && truth["tagged_count"] == json.Number("0"))
		audit, _ := cleanup["cleanup_audit"].(map[string]any)
		e.Metrics["contacts_cleanup_audit"] = audit
		add("contacts_cleanup_preserves_unowned_data", err == nil && audit["unowned_fields_unchanged"] == true && audit["groups_preserved"] == true && audit["containers_preserved"] == true && audit["whole_container_operations"] == json.Number("0") && audit["unowned_identifiers_before_sha256"] == audit["unowned_identifiers_after_sha256"])
	}()
	if _, err := fixture("create", map[string]any{}); err != nil {
		add("contact_fixture_created", false)
		return
	}
	token, err := w.ScopedToken(w.Gateway, "contacts:read,system:read")
	add("contacts_gateway_observer_scope", err == nil)
	if err != nil {
		return
	}
	endpoint := w.Gateway
	endpoint.Token = token
	observer, err := watch(ctx, endpoint, false, func(contracts.Record) bool { return true })
	add("contacts_gateway_observer_ready", err == nil)
	if err != nil {
		return
	}
	defer observer.close()
	input := relayProbeInput(w)
	input["Prefix"], input["Field"], input["Collection"] = tag, "job_title", "contacts.contact"
	var live struct {
		Values      []float64 `json:"latencies_ms"`
		Uncertainty float64   `json:"clock_uncertainty_ms"`
	}
	gateway := []float64{}
	err = w.RelayExchange("live", input, func() (any, error) {
		effects := []map[string]any{}
		for n := 0; n < 20; n++ {
			value := fmt.Sprintf("%s Testjob-%d", tag, n)
			startedWall := time.Now().UnixNano()
			effect, err := fixture("modify", map[string]any{"job_title": value})
			if err != nil {
				return nil, err
			}
			uptime, ok := effect["effect_uptime_ns"].(json.Number)
			if !ok {
				return nil, fmt.Errorf("contact effect clock missing")
			}
			ns, _ := uptime.Int64()
			wall, ok := effect["effect_wall_ns"].(json.Number)
			if !ok {
				return nil, fmt.Errorf("contact effect wall clock missing")
			}
			wallValue, clockErr := wall.Float64()
			if clockErr != nil || wallValue <= 0 {
				return nil, fmt.Errorf("invalid contact wall clock")
			}
			wallNS := int64(wallValue)
			effects = append(effects, map[string]any{"title": value, "wall_ns": wallNS, "started_wall_ns": startedWall})
			if err := w.Await(3*time.Second, func() bool { _, found := observer.observedContact(tag, value); return found }); err != nil {
				return nil, err
			}
			seen, _ := observer.observedContact(tag, value)
			gateway = append(gateway, max(0, float64(seen.UptimeNS-ns)/1e6))
		}
		return effects, nil
	}, &live)
	add("twenty_contacts_reach_public_replica", err == nil && len(live.Values) == 20 && len(gateway) == 20)
	e.Latency["contacts_effect_to_gateway"] = stats(gateway, 20-len(gateway))
	e.Latency["contacts_effect_to_public_replica"] = stats(live.Values, 20-len(live.Values))
	e.Metrics["contacts_clock_uncertainty_ms"] = live.Uncertainty
	add("contacts_direct_p95_under_one_second", len(gateway) == 20 && e.Latency["contacts_effect_to_gateway"].P95 <= 1000)
	add("contacts_public_p95_under_one_second", len(live.Values) == 20 && e.Latency["contacts_effect_to_public_replica"].P95+live.Uncertainty <= 1000)
	if err != nil {
		return
	}
	var crud contactsPublicCommands
	input["Tag"] = tag
	err = w.RelayProbe("contacts-crud", input, &crud)
	add("public_contacts_rest_and_mcp_crud", err == nil && len(crud.Values) == 6)
	if err != nil {
		return
	}
	contactsRelayCommandLatency(w, e, crud, add)
	truth, err := w.Truth("observe", map[string]any{"source": "contacts", "tag": tag})
	add("independent_public_command_cleanup", err == nil && truth["tagged_count"] == json.Number("1"))
}
