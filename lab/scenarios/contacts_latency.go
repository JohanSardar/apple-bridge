package scenarios

import (
	"encoding/json"
	"fmt"
	"strings"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func contactsLatencyEvidence(w *world.World, e *Evidence, observers []*watcher, effects []contactsEffect) {
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	events := readContactsTrace(w, e)
	scans := map[string]map[string]int64{}
	sent := map[string]contactsTrace{}
	commits := map[string][]contactsTrace{}
	counts := map[string]int{}
	unchanged := map[string]bool{}
	values := map[string][]float64{}
	preparation := map[string]map[string]int64{}
	sample := func(name string, from, through int64) {
		if from > 0 && through >= from {
			values[name] = append(values[name], float64(through-from)/1e6)
		}
	}
	for _, event := range events {
		if event.Stage == "gateway_commit" {
			commits[event.Checkpoint] = append(commits[event.Checkpoint], event)
		}
	}
	for _, event := range events {
		if strings.HasPrefix(event.Stage, "contacts_") {
			counts[event.Stage]++
			if event.Stage == "contacts_scan_started" {
				sample("contacts_notification_to_scan", event.NotificationNS, event.UptimeNS)
			}
			if event.ScanID != "" {
				if scans[event.ScanID] == nil {
					scans[event.ScanID] = map[string]int64{}
				}
				scans[event.ScanID][event.Stage] = event.UptimeNS
			}
			if event.Stage == "contacts_history_fetched" && !event.FullScan && !event.TokenChanged {
				unchanged[event.ScanID] = true
			}
			if event.Stage == "contacts_history_fetched" && event.FullScan {
				counts["full_scans"]++
			}
			if event.Stage == "contacts_capture_ready" && event.Empty {
				counts["empty_scans"]++
			}
		}
		if event.Stage == "helper_marker_written" {
			sample("helper_marker_write:"+event.Source, event.StartedNS, event.FinishedNS)
		}
		if event.Stage == "helper_blob_sent" {
			sample("helper_blob_queue:"+event.Source, event.QueuedNS, event.StartedNS)
			sample("helper_blob_encode:"+event.Source, event.StartedNS, event.PreparedNS)
			sample("helper_blob_socket_write:"+event.Source, event.PreparedNS, event.FinishedNS)
		}
		if event.Stage == "helper_blob_read" {
			sample("helper_blob_write:"+event.Source, event.PreparedNS, event.FinishedNS)
		}
		if event.Source != "" && event.ScanID != "" {
			if preparation[event.ScanID] == nil {
				preparation[event.ScanID] = map[string]int64{}
			}
			preparation[event.ScanID][event.Stage] = event.UptimeNS
			if event.Stage == "capture_written" {
				sample("helper_capture_write:"+event.Source, preparation[event.ScanID]["capture_sent"], event.UptimeNS)
			}
			if event.Stage == "capture_split" {
				sample("helper_split:"+event.Source, preparation[event.ScanID]["capture_received"], event.UptimeNS)
			}
			if event.Stage == "capture_enqueued" {
				sample("helper_chunk_prepare:"+event.Source, preparation[event.ScanID]["capture_chunk_preparing"], event.UptimeNS)
			}
		}
		if event.Source != "contacts" {
			continue
		}
		if event.Stage == "capture_received" {
			if scans[event.ScanID] == nil {
				scans[event.ScanID] = map[string]int64{}
			}
			scans[event.ScanID][event.Stage] = event.UptimeNS
		}
		if event.Stage == "capture_enqueued" {
			if scans[event.ScanID] == nil {
				scans[event.ScanID] = map[string]int64{}
			}
			scans[event.ScanID][event.Stage] = event.UptimeNS
		}
		if event.Stage == "capture_sent" {
			sample("contacts_publish_queue", scans[event.ScanID]["capture_enqueued"], event.UptimeNS)
			sent[event.Checkpoint] = event
		}
		if event.Stage == "committed_ack" {
			begin := sent[event.Checkpoint]
			sample("contacts_publish_to_ack", begin.UptimeNS, event.UptimeNS)
			matched := false
			for _, commit := range commits[event.Checkpoint] {
				if commit.CommitStartedWallNS < begin.WallNS || commit.WallNS > event.WallNS {
					continue
				}
				sample("contacts_publish_to_commit_start", int64(begin.WallNS), int64(commit.CommitStartedWallNS))
				sample("contacts_gateway_commit", int64(commit.CommitStartedWallNS), int64(commit.WallNS))
				matched = true
				break
			}
			if !matched {
				counts["publications_without_matched_commit_span"]++
			}
		}
	}
	unnecessary := 0
	for id, stages := range scans {
		if unchanged[id] && stages["capture_enqueued"] != 0 {
			unnecessary++
		}
		sample("contacts_ready_to_enqueue", stages["contacts_capture_ready"], stages["capture_enqueued"])
		sample("contacts_actor_queue", stages["contacts_capture_ready"], stages["capture_received"])
		sample("contacts_scan_queue", stages["contacts_scan_requested"], stages["contacts_scan_started"])
		sample("contacts_history_fetch", stages["contacts_scan_started"], stages["contacts_history_fetched"])
		fetched := stages["contacts_contact_fetched"]
		if fetched == 0 {
			fetched = stages["contacts_fetched"]
		}
		sample("contacts_fetch", stages["contacts_history_fetched"], fetched)
		sample("contacts_materialize", fetched, stages["contacts_encoded"])
		sample("contacts_hash", stages["contacts_encoded"], stages["contacts_hashed"])
		sample("contacts_compare", stages["contacts_encoded"], stages["contacts_compared"])
		sample("contacts_ready_to_commit", stages["contacts_capture_ready"], stages["contacts_capture_committed"])
	}
	bulkInFlight := map[string]bool{}
	bulkPeak := 0
	for _, event := range events {
		switch event.Stage {
		case "capture_session_disconnected":
			clear(bulkInFlight)
		case "capture_sent":
			if !event.Live {
				bulkInFlight[event.Checkpoint] = true
				if len(bulkInFlight) > bulkPeak {
					bulkPeak = len(bulkInFlight)
				}
			}
		case "committed_ack":
			delete(bulkInFlight, event.Checkpoint)
		}
	}
	counts["bulk_capture_peak_in_flight"] = bulkPeak
	if e.Scenario == "CON-EXTERNAL" && e.Level != "sim" {
		e.Assertions = append(e.Assertions, Assertion{Name: "bulk_capture_does_not_queue_ahead_of_live", Passed: bulkPeak <= 1})
		e.Passed = e.Passed && bulkPeak <= 1
	}
	groupChanges, groupFallbacks := 0, 0
	for _, event := range events {
		if event.Stage != "contacts_history_fetched" {
			continue
		}
		groupChanges += event.ChangedGroups
		for _, kind := range event.FullScanEventKinds {
			switch kind {
			case "CNChangeHistoryAddGroupEvent", "CNChangeHistoryUpdateGroupEvent", "CNChangeHistoryDeleteGroupEvent", "CNChangeHistoryAddMemberToGroupEvent", "CNChangeHistoryRemoveMemberFromGroupEvent", "CNChangeHistoryAddSubgroupToGroupEvent", "CNChangeHistoryRemoveSubgroupFromGroupEvent":
				groupFallbacks++
			}
		}
	}
	counts["incremental_group_identifiers"] = groupChanges
	counts["group_events_causing_full_contact_scan"] = groupFallbacks
	if e.Scenario == "CON-EXTERNAL" && e.Level != "sim" {
		e.Assertions = append(e.Assertions, Assertion{Name: "group_history_does_not_force_full_contact_refetch", Passed: groupFallbacks == 0})
		e.Passed = e.Passed && groupFallbacks == 0
	}
	contactsCommandLatency(w, e, events, observers)
	if e.Scenario == "CON-CRUD" && len(effects) == 0 {
		for _, event := range events {
			if event.Stage == "contacts_effect_done" {
				effects = append(effects, contactsEffect{At: event.UptimeNS})
			}
		}
	}
	contactsNotificationLatency(e, events, effects)
	if len(effects) > 0 {
		rawEffects := []any{}
		for _, effect := range effects {
			rawEffects = append(rawEffects, map[string]any{"effect_uptime_ns": json.Number(fmt.Sprint(effect.At))})
		}
		nativeMediaEvidence(w, e, rawEffects)
	}
	if _, exists := counts["contacts_scan_abandoned"]; !exists {
		counts["contacts_scan_abandoned"] = 0
	}
	counts["unchanged_history_scans"] = len(unchanged)
	counts["unchanged_history_publications"] = unnecessary
	e.Metrics["contacts_scan_counts"] = counts
	if e.Scenario == "CON-EXTERNAL" && e.Level != "sim" {
		ok := unnecessary == 0
		e.Assertions = append(e.Assertions, Assertion{Name: "unchanged_contacts_history_does_not_publish", Passed: ok})
		e.Passed = e.Passed && ok
	}
	e.Metrics["contacts_stage_scope"] = "all scans recorded before World teardown"
	for name, samples := range values {
		e.Latency[name] = stats(samples, 0)
	}
}
