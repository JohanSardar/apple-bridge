package scenarios

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func contactsCommandLatency(w *world.World, e *Evidence, events []contactsTrace, observers []*watcher) {
	if e.Scenario != "CON-CRUD" || e.Level == "sim" {
		return
	}
	identifiers, err := contactsRecordIdentifiers(w)
	for i, observer := range observers {
		values := []float64{}
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
