package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"reflect"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

const contactPNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII="

func runContacts(ctx context.Context, s Scenario, level string) (result Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	e.HostTelemetry = map[string]string{}
	for key, args := range map[string][]string{"load_average": {"sysctl", "-n", "vm.loadavg"}, "logical_cpus": {"sysctl", "-n", "hw.logicalcpu"}, "heavy_jobs": {"python3", "-c", `import glob; print(len(glob.glob("/tmp/apple-bridge-heavy-*.lock/pid")))`}} {
		value, err := exec.CommandContext(ctx, args[0], args[1:]...).Output()
		if err == nil {
			e.HostTelemetry[key] = strings.TrimSpace(string(value))
		}
	}
	if level == "replica" {
		e.Hosts = append(e.Hosts, "ubuntu")
	}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	fail := func(name string) Evidence { add(name, false); return e }
	native := level == "real" || level == "replica"
	w, err := world.NewContacts(ctx, level, 1, native, s.ID == "CON-HANDLE-LINK")
	if err != nil {
		return fail("world_start")
	}
	defer w.Close()
	tag := "[ab-test " + contracts.NewID("contacts") + "]"
	synthetic := map[string]any{"given_name": tag + " Testperson", "family_name": "Testfamily", "organization": "Bridge Test", "job_title": "Testjob", "phones": []any{map[string]any{"label": "mobile", "value": "+15555550123"}}, "emails": []any{map[string]any{"label": "home", "value": "bridge-test@example.com"}}}
	truth := func() (map[string]any, error) {
		return w.Truth("observe", map[string]any{"source": "contacts", "tag": tag})
	}
	lastEffect := int64(0)
	fixture := func(action string, fields map[string]any) error {
		lastEffect = world.UptimeNS()
		if native {
			fields["source"] = "contacts"
			fields["tag"] = tag
			fields["action"] = action
			response, err := w.Truth("fixture", fields)
			if value, ok := response["effect_uptime_ns"].(json.Number); ok {
				lastEffect, _ = value.Int64()
			}
			return err
		}
		if action == "modify" {
			action = "update"
		}
		request := map[string]any{"action": action, "key": tag, "data": fields}
		if action == "create" {
			request["data"] = synthetic
		}
		if action == "readonly" {
			request["action"] = "create"
			request["data"] = synthetic
			request["readonly"] = true
		}
		return w.ControlAction("/contacts", request)
	}
	if native {
		if _, err = truth(); err != nil {
			return fail("independent_contacts_truth_access")
		}
		defer func() {
			_, err := w.Truth("fixture", map[string]any{"source": "contacts", "action": "cleanup", "tag": tag})
			remaining, readErr := truth()
			ok := err == nil && readErr == nil && remaining["tagged_count"] == json.Number("0")
			result.Assertions = append(result.Assertions, Assertion{Name: "owned_test_contacts_cleaned", Passed: ok})
			result.Passed = result.Passed && ok
			empty := readErr == nil && remaining["bridge_test_count"] == json.Number("0")
			result.Assertions = append(result.Assertions, Assertion{Name: "bridge_test_organization_empty", Passed: empty})
			result.Passed = result.Passed && empty

		}()
	}
	find := func(endpoint world.Endpoint) (contracts.Record, bool) {
		records, err := contactRecords(ctx, w, endpoint)
		if err != nil {
			return contracts.Record{}, false
		}
		for _, record := range records {
			var data map[string]any
			json.Unmarshal(record.Data, &data)
			if record.Collection == "contacts.contact" && data["given_name"] == tag+" Testperson" && data["organization"] == "Bridge Test" {
				return record, true
			}
		}
		return contracts.Record{}, false
	}
	if fixture("create", map[string]any{}) != nil {
		return fail("fixture_create")
	}
	if w.Await(15*time.Second, func() bool { _, ok := find(w.Gateway); return ok }) != nil {
		return fail("unified_contacts_imported")
	}
	add("unified_contacts_imported", true)
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	observers := []*watcher{}
	for _, endpoint := range endpoints {
		observer, err := watch(ctx, endpoint, false, func(record contracts.Record) bool {
			if record.Collection != "contacts.contact" {
				return false
			}
			if native {
				aliases, err := gatewayAliases(w)
				if err != nil {
					return false
				}
				nativeKey := ""
				for key, id := range aliases {
					if id == record.ID && strings.HasPrefix(key, "contacts/contacts.contact/") {
						nativeKey = strings.TrimPrefix(key, "contacts/contacts.contact/")
						break
					}
				}
				if nativeKey == "" {
					return false
				}
				digest := sha256.Sum256([]byte(nativeKey))
				state, err := w.Truth("observe", map[string]any{"source": "contacts", "tag": tag, "identifier_hash": hex.EncodeToString(digest[:])})
				return err == nil && state["identifier_exists"] == false
			}
			return truthAbsent(w, record)
		})
		if err != nil {
			return fail("sse_observer")
		}
		defer observer.close()
		observers = append(observers, observer)
	}
	submit := func(protocol, operation string, args map[string]any, version uint64) (contracts.CommandRecord, error) {
		_, err := w.Cursor(w.Gateway)
		if err != nil {
			return contracts.CommandRecord{}, err
		}
		endpoint := w.Gateway
		if level == "replica" {
			endpoint = w.Replicas[0]
		}
		client := scripted.Client{Endpoint: endpoint, Protocol: protocol}
		var command contracts.CommandRecord
		err = client.Call(ctx, operation, map[string]any{"command_id": contracts.NewCommandID(), "args": args, "expected_version": version, "wait_ms": 15000}, &command)
		return command, err
	}
	switch s.ID {
	case "CON-IMPORT":
		if !native {
			if fixture("group", map[string]any{}) != nil {
				return fail("group_fixture")
			}
			if w.ControlAction("/contacts", map[string]any{"action": "create", "key": "linked-test-native", "unified_key": tag, "data": map[string]any{"emails": []any{map[string]any{"label": "work", "value": "linked-test@example.com"}}}}) != nil {
				return fail("linked_native_fixture")
			}
			record, ok := find(w.Gateway)
			var fields map[string]any
			json.Unmarshal(record.Data, &fields)
			emails, _ := fields["emails"].([]any)
			add("linked_contacts_are_unified", ok && len(emails) == 2)
		}
		if native {
			state, err := truth()
			if err != nil {
				return fail("independent_import_truth")
			}
			records, err := contactRecords(ctx, w, w.Gateway)
			if err != nil {
				return fail("import_records")
			}
			count := 0
			coverage := map[string]int{}
			mapping := map[string]string{"givenName": "given_name", "middleName": "middle_name", "familyName": "family_name", "previousFamilyName": "previous_family_name", "namePrefix": "name_prefix", "nameSuffix": "name_suffix", "nickname": "nickname", "phoneticGivenName": "phonetic_given_name", "phoneticMiddleName": "phonetic_middle_name", "phoneticFamilyName": "phonetic_family_name", "phoneticOrganizationName": "phonetic_organization", "organizationName": "organization", "jobTitle": "job_title", "departmentName": "department", "phoneNumbers": "phones", "emailAddresses": "emails", "postalAddresses": "postal_addresses", "urlAddresses": "urls", "birthday": "birthday", "nonGregorianBirthday": "non_gregorian_birthday", "dates": "dates", "socialProfiles": "social_profiles", "instantMessageAddresses": "instant_messages", "contactRelations": "relations", "imageData": "image", "thumbnailImageData": "thumbnail_image"}
			for _, record := range records {
				if record.Collection != "contacts.contact" {
					continue
				}
				count++
				var data map[string]any
				json.Unmarshal(record.Data, &data)
				add("note_entitlement_gap_explicit", data["note"] == "unavailable")
				for key, field := range mapping {
					value := data[field]
					present := value != nil
					if text, ok := value.(string); ok {
						present = text != ""
					}
					if values, ok := value.([]any); ok {
						present = len(values) > 0
					}
					if present {
						coverage[key]++
					}
				}
			}
			aliases, aliasErr := gatewayAliases(w)
			nativeKeys := map[string]string{}
			for key, id := range aliases {
				if strings.HasPrefix(key, "contacts/") {
					parts := strings.SplitN(key, "/", 3)
					if len(parts) == 3 {
						nativeKeys[id] = parts[2]
					}
				}
			}
			digest := func(value string) string { sum := sha256.Sum256([]byte(value)); return hex.EncodeToString(sum[:]) }
			groupTruth, _ := state["groups"].(map[string]any)
			groupCount := 0
			for _, record := range records {
				if record.Collection != "contacts.group" {
					continue
				}
				groupCount++
				var fields struct {
					Name    string   `json:"name"`
					Members []string `json:"member_ids"`
				}
				json.Unmarshal(record.Data, &fields)
				nativeGroup := nativeKeys[record.ID]
				expected, _ := groupTruth[digest(nativeGroup)].(map[string]any)
				actual := []string{}
				for _, id := range fields.Members {
					actual = append(actual, digest(nativeKeys[id]))
				}
				slices.Sort(actual)
				raw, _ := json.Marshal(actual)
				other, _ := json.Marshal(expected["member_digests"])
				add("group_name_and_membership_hashes", aliasErr == nil && nativeGroup != "" && expected["name_digest"] == digest(fields.Name) && string(raw) == string(other))
			}
			add("group_count_equals_truth", len(groupTruth) == groupCount)
			add("total_unified_count_equals_truth", state["total_count"] == json.Number(fmt.Sprint(count)))
			nativeCoverage, _ := state["coverage_counts"].(map[string]any)
			for key := range mapping {
				add("field_coverage_"+key, nativeCoverage[key] == json.Number(fmt.Sprint(coverage[key])))
			}
		} else {
			add("sim_truth_equal", w.Await(5*time.Second, func() bool { return contactTruthConvergence(ctx, w) }) == nil)
		}
	case "CON-EXTERNAL":
		values := make([][]float64, len(endpoints))
		for n := 0; n < 20; n++ {
			value := fmt.Sprintf("Testjob-%d", n)
			started := time.Now()
			if fixture("modify", map[string]any{"job_title": value}) != nil {
				return fail("external_modify")
			}
			effect := lastEffect
			for i, observer := range observers {
				if w.Await(3*time.Second, func() bool { _, ok := observer.observedContact(tag, value); return ok }) != nil {
					return fail("external_visible")
				}
				seen, ok := observer.observedContact(tag, value)
				if !ok || effect <= 0 || seen.UptimeNS < effect {
					return fail("monotonic_clock_agreement")
				}
				values[i] = append(values[i], float64(seen.UptimeNS-effect)/float64(time.Millisecond))
			}
			add("bounded_fixture_execution", time.Since(started) < 10*time.Second)
		}
		for i, samples := range values {
			name := "effect_done→gateway_visible"
			if i > 0 {
				name = fmt.Sprintf("effect_done→replica_visible:replica%d", i-1)
			}
			e.Latency[name] = stats(samples, 0)
			add("latency_"+name, e.Latency[name].P95 < 1000)
		}
	case "CON-CRUD":
		if fixture("delete", map[string]any{}) != nil {
			return fail("remove_external_fixture")
		}
		if w.Await(3*time.Second, func() bool { _, ok := find(w.Gateway); return !ok }) != nil {
			return fail("fixture_absence")
		}
		for _, protocol := range []string{"rest", "mcp"} {
			command, err := submit(protocol, "contacts.create", synthetic, 0)
			if err != nil || command.Disposition != "locally_observed" || command.CommitSeq == 0 {
				return fail(protocol + "_create_evidence")
			}
			add(protocol+"_create_evidence", true)
			for _, operation := range []string{"contacts.update", "contacts.delete"} {
				var record contracts.Record
				if w.Await(5*time.Second, func() bool { var ok bool; record, ok = find(w.Gateway); return ok }) != nil {
					return fail("command_target")
				}
				args := map[string]any{"id": record.ID}
				if operation == "contacts.update" {
					args["job_title"] = "Updated Testjob"
				}
				command, err = submit(protocol, operation, args, record.Version)
				if err != nil || command.Disposition != "locally_observed" || command.CommitSeq == 0 {
					return fail(protocol + "_" + operation)
				}
				add(protocol+"_"+operation, true)
				if w.Await(5*time.Second, func() bool {
					for _, endpoint := range endpoints {
						current, ok := find(endpoint)
						if operation == "contacts.delete" {
							if ok {
								return false
							}
							continue
						}
						var fields map[string]any
						json.Unmarshal(current.Data, &fields)
						if !ok || fields["job_title"] != "Updated Testjob" {
							return false
						}
					}
					return true
				}) != nil {
					return fail("read_your_write")
				}
				if native {
					state, err := truth()
					if err != nil {
						return fail("truth_crud")
					}
					expected := json.Number("1")
					if operation == "contacts.delete" {
						expected = "0"
					}
					add(protocol+"_truth_"+operation, state["tagged_count"] == expected)
					if operation == "contacts.update" {
						records, _ := state["records"].([]any)
						if len(records) != 1 {
							return fail("truth_unique")
						}
						fields, _ := records[0].(map[string]any)
						add(protocol+"_truth_patch", fields["job_title"] == "Updated Testjob")
					}
				}
			}
		}

	case "CON-RESOLVE-RESTART":
		before, _ := find(w.Gateway)
		if w.ControlAction("/fault", map[string]any{"name": "ambiguous-outcome"}) != nil {
			return fail("ambiguous_contact_fault")
		}
		command, err := submit("rest", "contacts.update", map[string]any{"id": before.ID, "job_title": "Resolved Testjob"}, before.Version)
		if err != nil || command.Disposition != "indeterminate" {
			return fail("contact_outcome_indeterminate")
		}
		if w.RestartGateway() != nil {
			return fail("gateway_restart")
		}
		if w.Await(5*time.Second, func() bool {
			var resolved contracts.CommandRecord
			_, err := world.Request(ctx, w.Gateway, "/v1/op/commands.status", contracts.CommandQuery{ID: command.Command.ID, Resolve: true}, &resolved)
			return err == nil && resolved.Disposition == "locally_observed" && resolved.CommitSeq > 0
		}) != nil {
			return fail("same_history_token_resolves_after_restart")
		}
		current, ok := find(w.Gateway)
		add("reobserved_token_does_not_duplicate_version", ok && current.Version == before.Version+1)
		effects, err := w.Effects()
		add("resolve_never_redispatches", err == nil && effects[command.Command.ID] == 1)
	case "CON-PERMISSION-LOSS":
		before, _ := find(w.Gateway)
		if w.ControlAction("/source-state", map[string]string{"source": "contacts", "state": "permission_missing"}) != nil {
			return fail("permission_loss_fault")
		}
		if fixture("delete", map[string]any{}) != nil {
			return fail("permission_denied_external_effect")
		}
		add("permission_missing_reported", w.Await(3*time.Second, func() bool {
			var status contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/sources.status", struct{}{}, &status)
			if err != nil {
				return false
			}
			for _, record := range status.Records {
				var fields map[string]any
				json.Unmarshal(record.Data, &fields)
				if fields["source"] == "contacts" && fields["state"] == "permission_missing" {
					return true
				}
			}
			return false
		}) == nil)
		after, ok := find(w.Gateway)
		add("permission_loss_preserves_record", ok && reflect.DeepEqual(before, after))
	case "CON-CONFLICT":
		before, _ := find(w.Gateway)
		if fixture("modify", map[string]any{"job_title": "External Testjob"}) != nil {
			return fail("conflict_external_fixture")
		}
		_, err := submit("rest", "contacts.update", map[string]any{"id": before.ID, "job_title": "Forbidden Testjob"}, before.Version)
		add("stale_expected_version_conflicts", err != nil)
		effects, err := w.Effects()
		add("stale_version_zero_effects", err == nil && len(effects) == 0)
		fresh, _ := find(w.Gateway)
		var hidden map[string]any
		json.Unmarshal(fresh.Data, &hidden)
		hidden["job_title"] = "Hidden External Testjob"
		if w.ControlAction("/external-change", map[string]any{"key": tag, "collection": "contacts.contact", "data": hidden, "omit_capture": true}) != nil {
			return fail("unreported_native_edit")
		}
		_, err = submit("rest", "contacts.update", map[string]any{"id": fresh.ID, "job_title": "Forbidden Testjob"}, fresh.Version)
		var conflict *contracts.Error
		add("fresh_native_refetch_conflict_with_current", errors.As(err, &conflict) && conflict.Code == "conflict" && conflict.Current != nil && conflict.Current.ID == fresh.ID && conflict.Current.Version > fresh.Version)
		fresh, _ = find(w.Gateway)
		if w.ControlAction("/external-change", map[string]any{"key": tag, "collection": "contacts.contact", "delete": true, "omit_capture": true}) != nil {
			return fail("unreported_native_delete")
		}
		_, err = submit("rest", "contacts.update", map[string]any{"id": fresh.ID, "job_title": "Forbidden Testjob"}, fresh.Version)
		conflict = nil
		add("fresh_native_absence_conflict_with_current", errors.As(err, &conflict) && conflict.Code == "conflict" && conflict.Current != nil && conflict.Current.Deleted)
		effects, err = w.Effects()
		add("native_races_zero_effects", err == nil && len(effects) == 0)

	case "CON-DROP-EVERYTHING":
		before, _ := find(w.Gateway)
		var original contracts.ReadResult
		_, err = world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "contacts.contact", ID: before.ID, IncludeAliases: true}, &original)
		oldGeneration := ""
		for _, alias := range original.Aliases {
			if alias.NativeKey == tag {
				oldGeneration = alias.Generation
			}
		}
		if err != nil || oldGeneration == "" {
			return fail("history_generation_before_reset")
		}
		if fixture("drop-everything", map[string]any{}) != nil {
			return fail("history_reset")
		}
		after, ok := find(w.Gateway)
		add("generation_reset_preserves_identity_and_version", ok && before.ID == after.ID && before.Version == after.Version)
		var reset contracts.ReadResult
		_, err = world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: "contacts.contact", ID: before.ID, IncludeAliases: true}, &reset)
		generations := map[string]bool{}
		for _, alias := range reset.Aliases {
			if alias.NativeKey == tag {
				generations[alias.Generation] = true
			}
		}
		add("history_reset_changes_generation", err == nil && len(generations) == 1 && !generations[oldGeneration])

	case "CON-TOKEN-CRASH":
		before, _ := find(w.Gateway)
		_ = fixture("token-crash", map[string]any{"job_title": "Crash Testjob"})
		if w.RestartHelper() != nil {
			return fail("restart_after_history_fetch")
		}
		if w.Await(5*time.Second, func() bool {
			record, ok := find(w.Gateway)
			var fields map[string]any
			json.Unmarshal(record.Data, &fields)
			return ok && fields["job_title"] == "Crash Testjob"
		}) != nil {
			return fail("history_change_replayed")
		}
		after, _ := find(w.Gateway)
		add("exactly_one_record_version", after.ID == before.ID && after.Version == before.Version+1)
	case "CON-READONLY-CONTAINER":
		if fixture("readonly", map[string]any{}) != nil {
			return fail("readonly_fixture")
		}
		record, _ := find(w.Gateway)
		command, err := submit("rest", "contacts.update", map[string]any{"id": record.ID, "job_title": "Forbidden Testjob"}, record.Version)
		add("readonly_rejected_before_effect", err != nil && command.Disposition != "locally_observed")
		effects, err := w.Effects()
		add("readonly_zero_effects", err == nil && len(effects) == 0)
	case "CON-IMAGE":
		if fixture("image", map[string]any{"base64": contactPNG}) != nil {
			return fail("image_fixture")
		}
		for _, endpoint := range endpoints {
			verified := w.Await(10*time.Second, func() bool {
				record, ok := find(endpoint)
				if !ok {
					return false
				}
				var data struct {
					Image struct {
						Digest string `json:"digest"`
					} `json:"image"`
				}
				json.Unmarshal(record.Data, &data)
				if data.Image.Digest == "" {
					return false
				}
				req, err := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/blobs/"+data.Image.Digest+"?record="+record.ID, nil)
				if err != nil {
					return false
				}
				req.Host = endpoint.Host
				req.Header.Set("Authorization", "Bearer "+endpoint.Token)
				response, err := http.DefaultClient.Do(req)
				if err != nil {
					return false
				}
				defer response.Body.Close()
				bytes, _ := io.ReadAll(response.Body)
				digest := sha256.Sum256(bytes)
				return response.StatusCode == 200 && hex.EncodeToString(digest[:]) == data.Image.Digest
			})
			add("image_blob_authorized_and_digest_verified", verified == nil)
		}
	case "CON-HANDLE-LINK":
		var lookup contracts.ReadResult
		_, err := world.Request(ctx, w.Gateway, "/v1/op/contacts.lookup", contracts.ContactLookup{Handle: "+1 (555) 555-0123"}, &lookup)
		add("normalized_handle_lookup", err == nil && len(lookup.Records) > 0)
		if !native {
			if w.ControlAction("/external-change", map[string]any{"key": "test-handle", "collection": "messages.handle", "data": map[string]any{"address": "+1 (555) 555-0123", "service": "iMessage"}}) != nil {
				return fail("handle_fixture")
			}
		}
		add("message_handle_contact_link", w.Await(5*time.Second, func() bool {
			records, err := contactRecords(ctx, w, w.Gateway)
			if err != nil {
				return false
			}
			for _, record := range records {
				if record.Collection != "messages.handle" {
					continue
				}
				var fields map[string]any
				json.Unmarshal(record.Data, &fields)
				if fields["contact_id"] != nil {
					return true
				}
			}
			return false
		}) == nil)
		if !native {
			add("handle_link_journaled", w.Await(3*time.Second, func() bool {
				observers[0].mu.Lock()
				defer observers[0].mu.Unlock()
				for _, seen := range observers[0].seen {
					if seen.Change.Record.Collection != "messages.handle" {
						continue
					}
					var data map[string]any
					json.Unmarshal(seen.Change.Record.Data, &data)
					if data["contact_id"] != nil {
						return true
					}
				}
				return false
			}) == nil)
			linked := func() bool {
				records, err := contactRecords(ctx, w, w.Gateway)
				if err != nil {
					return false
				}
				for _, record := range records {
					if record.Collection != "messages.handle" {
						continue
					}
					var fields map[string]any
					json.Unmarshal(record.Data, &fields)
					if fields["contact_id"] != nil {
						return true
					}
				}
				return false
			}
			if fixture("modify", map[string]any{"phones": []any{}, "emails": []any{}}) != nil {
				return fail("contact_handles_removed")
			}
			add("contact_edit_clears_link", w.Await(3*time.Second, func() bool { return !linked() }) == nil)
			if fixture("modify", map[string]any{"phones": synthetic["phones"], "emails": synthetic["emails"]}) != nil {
				return fail("contact_handles_restored")
			}
			add("contact_edit_restores_link", w.Await(3*time.Second, linked) == nil)
			if w.ControlAction("/contacts", map[string]any{"action": "create", "key": tag + "-duplicate", "data": synthetic}) != nil {
				return fail("ambiguous_contact_fixture")
			}
			add("ambiguous_handles_unlinked", w.Await(3*time.Second, func() bool { return !linked() }) == nil)
			if w.ControlAction("/contacts", map[string]any{"action": "delete", "key": tag + "-duplicate"}) != nil {
				return fail("ambiguous_contact_removed")
			}
			add("unambiguous_link_restored", w.Await(3*time.Second, linked) == nil)
			for _, endpoint := range endpoints {
				token, err := w.ScopedToken(endpoint, "messages:read")
				if err != nil {
					return fail("messages_scope_token")
				}
				endpoint.Token = token
				var hidden contracts.ReadResult
				_, err = world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: "messages.handle"}, &hidden)
				masked := err == nil && len(hidden.Records) > 0
				for _, record := range hidden.Records {
					var fields map[string]any
					json.Unmarshal(record.Data, &fields)
					if fields["contact_id"] != nil {
						masked = false
					}
				}
				add("contact_link_hidden_without_contacts_scope", masked)
				scopedObserver, err := watch(ctx, endpoint, false, func(contracts.Record) bool { return false })
				if err != nil {
					return fail("messages_scope_observer")
				}
				defer scopedObserver.close()
				if w.ControlAction("/external-change", map[string]any{"key": "test-handle", "collection": "messages.handle", "data": map[string]any{"address": "+1 (555) 555-0123", "service": contracts.NewID("test")}}) != nil {
					return fail("scoped_handle_change")
				}
				add("contact_link_hidden_in_sse", w.Await(3*time.Second, func() bool {
					scopedObserver.mu.Lock()
					defer scopedObserver.mu.Unlock()
					for _, seen := range scopedObserver.seen {
						if seen.Change.Record.Collection != "messages.handle" {
							continue
						}
						var fields map[string]any
						json.Unmarshal(seen.Change.Record.Data, &fields)
						return fields["contact_id"] == nil
					}
					return false
				}) == nil)
			}

		}

	}
	add("replica_convergence", w.Await(10*time.Second, func() bool {
		gateway, err := contactRecords(ctx, w, w.Gateway)
		if err != nil {
			return false
		}
		for _, endpoint := range w.Replicas {
			replica, err := contactRecords(ctx, w, endpoint)
			if err != nil || !reflect.DeepEqual(withoutCommands(gateway), withoutCommands(replica)) {
				return false
			}
		}
		return true
	}) == nil)
	if !native && s.ID != "CON-PERMISSION-LOSS" {
		add("independent_sim_truth", w.Await(5*time.Second, func() bool { return contactTruthConvergence(ctx, w) }) == nil)
	}
	for i, observer := range observers {
		observer.mu.Lock()
		valid := !observer.invalid
		noDeletion := !observer.falseDelete
		observer.mu.Unlock()
		add(fmt.Sprintf("sse_contiguous_%d", i), valid)
		add(fmt.Sprintf("no_false_deletion_%d", i), noDeletion)
	}
	scoped, err := w.ScopedToken(w.Gateway, "system:read")
	if err != nil {
		return fail("scoped_token")
	}
	endpoint := w.Gateway
	endpoint.Token = scoped
	var hidden contracts.ReadResult
	_, err = world.Request(ctx, endpoint, "/v1/op/contacts.lookup", contracts.ContactLookup{}, &hidden)
	add("contacts_scope_enforced", err == nil && len(hidden.Records) == 0)
	recordIntegrity(&e, w, nil)
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && (assertion.Passed || assertion.Status == "unverified")
	}
	return e
}

func contactTruthConvergence(ctx context.Context, w *world.World) bool {
	observations, err := w.Inspect()
	if err != nil {
		return false
	}
	aliases, err := gatewayAliases(w)
	if err != nil {
		return false
	}
	filtered := []contracts.Observation{}
	for _, observation := range observations {
		source := contracts.CollectionSource(observation.Collection)
		if source == contracts.Contacts || source == contracts.Messages {
			filtered = append(filtered, observation)
		}
	}
	expected, err := truthRecords(filtered, aliases)
	if err != nil {
		return false
	}
	actual, err := contactRecords(ctx, w, w.Gateway)
	if err != nil {
		return false
	}
	for i, r := range actual {
		if r.Collection == "messages.handle" {
			var data map[string]any
			json.Unmarshal(r.Data, &data)
			delete(data, "contact_id")
			actual[i].Data, _ = json.Marshal(data)
		}
	}
	return reflect.DeepEqual(recordMultiset(actual, false), recordMultiset(expected, false))
}

func (w *watcher) observedContact(tag, job string) (Seen, bool) {
	w.mu.Lock()
	defer w.mu.Unlock()
	for _, seen := range w.seen {
		if seen.Change.Record.Collection != "contacts.contact" {
			continue
		}
		var data map[string]any
		json.Unmarshal(seen.Change.Record.Data, &data)
		if data["given_name"] == tag+" Testperson" && data["job_title"] == job {
			return seen, true
		}
	}
	return Seen{}, false
}

func contactRecords(ctx context.Context, w *world.World, endpoint world.Endpoint) ([]contracts.Record, error) {
	records := []contracts.Record{}
	for _, collection := range []string{"contacts.contact", "contacts.group", "messages.handle"} {
		after := ""
		for {
			var page contracts.ReadResult
			_, err := world.Request(ctx, endpoint, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, Limit: 1000, After: after}, &page)
			if err != nil {
				return nil, err
			}
			records = append(records, page.Records...)
			if page.Next == "" {
				break
			}
			after = page.Next
		}
	}
	return records, nil
}
