package scenarios

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"slices"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func contactsCommandLatency(w *world.World, e *Evidence, events []contactsTrace, observers []*watcher) {
	if e.Scenario != "CON-CRUD" || e.Level == "sim" {
		return
	}
	contactsCommandStages(e, events)
	identifiers, err := contactsRecordIdentifiers(w)
	for i, observer := range observers {
		values := []float64{}
		delivery := []float64{}
		effects := 0
		observer.mu.Lock()
		for _, event := range events {
			if event.Stage != "contacts_effect_done" || len(event.Keys) != 1 {
				continue
			}
			effects++
			for _, seen := range observer.changes {
				record := seen.Change.Record
				if record.ID != identifiers[event.Keys[0]] || seen.UptimeNS < event.UptimeNS {
					continue
				}
				if record.Deleted != (event.Operation == "contacts.delete") {
					continue
				}
				var data struct {
					Job string `json:"job_title"`
				}
				if event.Operation == "contacts.update" && (json.Unmarshal(record.Data, &data) != nil || data.Job != "Updated Testjob") {
					continue
				}
				values = append(values, float64(seen.UptimeNS-event.UptimeNS)/1e6)
				if committed := contactsCommandCommitUptime(events, event); committed > 0 && seen.UptimeNS >= committed {
					delivery = append(delivery, float64(seen.UptimeNS-committed)/1e6)
				}
				break
			}
		}
		observer.mu.Unlock()
		name := "contacts_command_effect_to_gateway_visible"
		if i > 0 {
			name = "contacts_command_effect_to_replica_visible"
		}
		latency := stats(values, effects-len(values))
		e.Latency[name] = latency
		e.Latency[name+"_after_commit"] = stats(delivery, effects-len(delivery))
		ok := err == nil && effects == 6 && len(values) == effects && latency.P95 <= 1000
		e.Assertions = append(e.Assertions, Assertion{Name: name + "_under_one_second", Passed: ok})
		e.Passed = e.Passed && ok
	}
}

type contactsEffect struct{ At int64 }

func contactsNotificationLatency(e *Evidence, events []contactsTrace, effects []contactsEffect) {
	delivery, scheduling := []float64{}, []float64{}
	effectTimes := []int64{}
	for _, effect := range effects {
		effectTimes = append(effectTimes, effect.At)
	}
	e.Metrics["contacts_effect_uptimes_ns"] = effectTimes
	for _, effect := range effects {
		var notified, started int64
		for _, event := range events {
			if event.UptimeNS < effect.At {
				continue
			}
			if event.Stage == "contacts_notified" && notified == 0 {
				notified = event.UptimeNS
			}
			if event.Stage == "contacts_scan_started" && notified != 0 {
				started = event.UptimeNS
				break
			}
		}
		if notified > 0 {
			delivery = append(delivery, float64(notified-effect.At)/1e6)
		}
		if started >= notified && notified > 0 {
			scheduling = append(scheduling, float64(started-notified)/1e6)
		}
	}
	if len(effects) > 0 {
		e.Latency["contacts_effect_to_next_notification"] = stats(delivery, len(effects)-len(delivery))
		e.Latency["contacts_next_notification_to_scan"] = stats(scheduling, len(effects)-len(scheduling))
	}
}

func contactsRecordIdentifiers(w *world.World) (map[string]string, error) {
	aliases, err := gatewayAliases(w)
	identifiers := map[string]string{}
	for alias, id := range aliases {
		if !strings.HasPrefix(alias, "contacts/contacts.contact/") {
			continue
		}
		digest := sha256.Sum256([]byte(strings.TrimPrefix(alias, "contacts/contacts.contact/")))
		identifiers[hex.EncodeToString(digest[:])] = id
	}
	return identifiers, err
}

func contactsCommandStages(e *Evidence, events []contactsTrace) {
	values := map[string][]float64{}
	sample := func(name string, from, through int64) {
		if from > 0 && through >= from {
			values[name] = append(values[name], float64(through-from)/1e6)
		}
	}
	commands := map[string]map[string]int64{}
	scans := map[string]map[string]int64{}
	for _, event := range events {
		if event.ScanID != "" {
			if scans[event.ScanID] == nil {
				scans[event.ScanID] = map[string]int64{}
			}
			scans[event.ScanID][event.Stage] = event.UptimeNS
		}
		if event.Operation == "" {
			continue
		}
		if commands[event.Operation] == nil {
			commands[event.Operation] = map[string]int64{}
		}
		stages := commands[event.Operation]
		stages[event.Stage] = event.UptimeNS
		switch event.Stage {
		case "contacts_prepared":
			sample("contacts_command_prepare", stages["contacts_prepare_started"], event.UptimeNS)
		case "contacts_save_started":
			sample("contacts_command_prepared_to_save", stages["contacts_prepared"], event.UptimeNS)
		case "contacts_effect_done":
			sample("contacts_command_save_execute", stages["contacts_save_started"], event.UptimeNS)
		case "contacts_effect_revalidated":
			sample("contacts_command_effect_revalidation", stages["contacts_effect_done"], event.UptimeNS)
		case "contacts_command_reconcile_requested":
			sample("contacts_command_effect_to_reconcile", stages["contacts_effect_done"], event.UptimeNS)
		}
	}
	for _, effect := range events {
		if effect.Stage != "contacts_effect_done" || len(effect.Keys) != 1 {
			continue
		}
		for _, sent := range events {
			if sent.Stage != "capture_sent" || sent.Source != "contacts" || sent.UptimeNS < effect.UptimeNS || !slices.Contains(sent.Keys, effect.Keys[0]) {
				continue
			}
			stages := scans[sent.ScanID]
			sample("contacts_command_effect_to_scan", effect.UptimeNS, stages["contacts_scan_started"])
			sample("contacts_command_scan_to_ready", stages["contacts_scan_started"], stages["contacts_capture_ready"])
			sample("contacts_command_ready_to_publish", stages["contacts_capture_ready"], sent.UptimeNS)
			for _, commit := range events {
				if commit.Stage == "gateway_commit" && commit.Checkpoint == sent.Checkpoint && commit.CommitStartedWallNS >= sent.WallNS {
					sample("contacts_command_publish_to_commit_start", int64(sent.WallNS), int64(commit.CommitStartedWallNS))
					sample("contacts_command_commit", int64(commit.CommitStartedWallNS), int64(commit.WallNS))
					break
				}
			}
			break
		}
	}
	for name, samples := range values {
		e.Latency[name] = stats(samples, 0)
	}
}

func contactsCommandCommitUptime(events []contactsTrace, effect contactsTrace) int64 {
	for _, sent := range events {
		if sent.Stage != "capture_sent" || sent.Source != "contacts" || sent.UptimeNS < effect.UptimeNS || !slices.Contains(sent.Keys, effect.Keys[0]) {
			continue
		}
		for _, commit := range events {
			if commit.Stage == "gateway_commit" && commit.Checkpoint == sent.Checkpoint && commit.CommitStartedWallNS >= sent.WallNS {
				return sent.UptimeNS + int64(commit.WallNS-sent.WallNS)
			}
		}
		break
	}
	return 0
}
