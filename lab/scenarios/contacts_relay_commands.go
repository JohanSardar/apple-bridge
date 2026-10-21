package scenarios

import "github.com/JohanSardar/apple-bridge/lab/world"

type contactsPublicCommands struct {
	Values []struct {
		Operation  string `json:"operation"`
		RecordID   string `json:"record_id"`
		WallNS     int64  `json:"wall_ns"`
		ReadWallNS int64  `json:"read_wall_ns"`
	} `json:"values"`
	Offset      float64 `json:"offset_ms"`
	Uncertainty float64 `json:"uncertainty_ms"`
}

func contactsRelayCommandLatency(w *world.World, e *Evidence, crud contactsPublicCommands, add func(string, bool)) {
	identifiers, err := contactsRecordIdentifiers(w)
	if err != nil {
		add("public_contacts_command_identity", false)
		return
	}
	events := readContactsTrace(w, e)
	samples, reads := []float64{}, []float64{}
	for _, visible := range crud.Values {
		for _, event := range events {
			if event.Stage != "contacts_effect_done" || len(event.Keys) != 1 || event.Operation != visible.Operation || identifiers[event.Keys[0]] != visible.RecordID {
				continue
			}
			latency := float64(visible.WallNS-int64(event.WallNS))/1e6 + crud.Offset
			if latency < -crud.Uncertainty {
				add("public_contacts_command_clock_agreement", false)
				return
			}
			samples = append(samples, max(0, latency))
			reads = append(reads, max(0, float64(visible.ReadWallNS-int64(event.WallNS))/1e6+crud.Offset))
			break
		}
	}
	e.Latency["contacts_command_effect_to_public_replica"] = stats(samples, 6-len(samples))
	e.Latency["contacts_command_effect_to_public_read"] = stats(reads, 6-len(reads))
	e.Metrics["contacts_command_clock_uncertainty_ms"] = crud.Uncertainty
	e.Metrics["contacts_command_visibility_measure"] = "native save through Ubuntu SSE receive; local REST and MCP reads independently verified"
	add("public_contacts_command_p95_under_one_second", len(samples) == 6 && e.Latency["contacts_command_effect_to_public_replica"].P95+crud.Uncertainty <= 1000)
}
