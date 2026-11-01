package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"maps"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"syscall"
	"time"
)

func runNotes(ctx context.Context, s Scenario, level string) (e Evidence) {
	if level != "sim" {
		restore := notesNativeBuildLock("native/.build")
		defer restore()
	}
	if s.ID == "NOTE-EDIT-PARITY" {
		return runNotesParity(ctx, s, level)
	}
	if s.ID == "NOTE-LIST" {
		return runNotesList(ctx, s, level)
	}
	if s.ID == "NOTE-EXTRAS" {
		return runNotesExtras(ctx, s, level)
	}
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	if s.ID == "NOTE-DECODE" || s.ID == "NOTE-DECODE-FULL" && level == "sim" {
		build := func(pkg, target string) (string, error) {
			cmd := exec.CommandContext(ctx, "packaging/dev/build-native.sh", pkg)
			cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NATIVE_LAB=1", "APPLE_BRIDGE_NATIVE_TARGET="+target)
			cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
			cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
			output, err := cmd.CombinedOutput()
			if err != nil {
				fmt.Fprintln(os.Stderr, string(output))
				return "", err
			}
			directory, err := os.ReadFile(filepath.Join(pkg, ".build/native-output-1"))
			return strings.TrimSpace(string(directory)), err
		}
		truth := ""
		if s.ID == "NOTE-DECODE-FULL" {
			directory, err := build("lab/truth/native", "AppleTruth")
			if err != nil {
				add("truth_build", false)
				return
			}
			truth = filepath.Join(directory, "AppleTruth")
		}
		directory, err := build("native", "NotesDecoderChecks")
		if err != nil {
			add("decoder_build", false)
			return
		}
		cmd := exec.CommandContext(ctx, filepath.Join(directory, "NotesDecoderChecks"), "lab/fixtures/notes")
		cmd.Env = append(os.Environ(), "APPLE_BRIDGE_NOTES_TRUTH="+truth)
		cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
		cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
		output, err := cmd.CombinedOutput()
		fmt.Fprintln(os.Stderr, strings.TrimSpace(string(output)))
		if s.ID == "NOTE-DECODE-FULL" {
			add("independent_truth_counts", err == nil)
		}
		add("gzip_protobuf_scrubbed_fixtures", err == nil)
		return
	}
	ctx = world.WithOptions(ctx, world.Options{GatewayDials: level == "replica", Scope: "notes:read,notes:write,reminders:read,system:read"})
	var w *world.World
	var err error
	if level == "sim" {
		w, err = world.New(ctx, level, 1, 0)
	} else {
		w, err = world.NewReal(ctx, level, 1)
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "notes world:", err)
		add("world_start", false)
		return
	}
	defer w.Close()
	if level == "real" && s.ID == "NOTE-CRUD" {
		before, problem := cleanupSnapshot(ctx, "notes")
		if problem != nil {
			e.Status = "limited"
			e.Assertions = append(e.Assertions, Assertion{Name: "cleanup_baseline", Status: "limited", Detail: "needs Notes Full Disk Access"})
			return
		}
		audit, problem := newRunCleanup(e.RunID, before)
		if problem != nil {
			add("cleanup_baseline", false)
			return
		}
		defer func() { finishCleanupAudit(context.Background(), audit, "notes", &e) }()
	}
	if (s.ID == "NOTE-IMPORT" || s.ID == "NOTE-DECODE-FULL") && level == "real" {
		response, truthErr := w.Truth("observe", map[string]any{"source": "notes", "sqlite": true})
		if truthErr != nil {
			add("independent_sqlite_copy", false)
			return
		}
		e.Metrics = map[string]any{}
		hydrationStarted := time.Now()
		pendingPeak, pendingLast := 0, 0
		statusCounts := map[string]int{}
		hydrationErr := w.Await(time.Minute, func() bool {
			records, readErr := w.Records(w.Gateway)
			if readErr != nil || len(records) == 0 {
				return false
			}
			pending, count := 0, 0
			clear(statusCounts)
			for _, record := range records {
				if record.Collection != "notes.note" || record.Deleted {
					continue
				}
				count++
				var data map[string]any
				if json.Unmarshal(record.Data, &data) != nil {
					return false
				}
				if data["body_status"] == "pending" {
					pending++
				}
				status, _ := data["content_status"].(string)
				switch status {
				case "complete", "partial", "offloaded", "undecoded", "locked":
					statusCounts[status]++
				default:
					statusCounts["unknown"]++
				}
			}
			if pending > pendingPeak {
				e.Metrics["hydration_pending_status_counts"] = maps.Clone(statusCounts)
			}
			pendingLast = pending
			pendingPeak = max(pendingPeak, pending)
			truthRows, _ := response["records"].([]any)
			return pending == 0 && count == len(truthRows)
		})
		e.Metrics["hydration_wait_ms"] = float64(time.Since(hydrationStarted).Microseconds()) / 1000
		e.Metrics["hydration_pending_peak"] = pendingPeak
		e.Metrics["hydration_pending_final"] = pendingLast
		e.Metrics["hydration_final_status_counts"] = statusCounts
		add("hydration_finished_before_comparison", hydrationErr == nil)
		if hydrationErr != nil {
			return
		}
		aliases, aliasErr := gatewayAliases(w)
		if aliasErr != nil {
			add("identity_map", false)
			return
		}
		add("independent_sqlite_copy", true)
		for _, entry := range []struct{ key, collection string }{{"folders", "notes.folder"}, {"records", "notes.note"}} {
			truthRows, _ := response[entry.key].([]any)
			reasons := map[string]bool{}
			mismatch := func(field, kind string) { reasons[field+":"+kind] = true }
			matches := w.Await(10*time.Second, func() bool {
				clear(reasons)
				aliases, _ = gatewayAliases(w)
				records, readErr := w.Records(w.Gateway)
				if readErr != nil {
					mismatch("records", "read_failed")
					return false
				}
				actual := map[string]contracts.Record{}
				for _, record := range records {
					if record.Collection == entry.collection && !record.Deleted {
						actual[record.ID] = record
					}
				}
				if len(actual) != len(truthRows) {
					mismatch("records", "count")
					return false
				}
				for _, value := range truthRows {
					native := value.(map[string]any)
					id := aliases[aliasKey(contracts.Notes, entry.collection, native["id"].(string))]
					record, ok := actual[id]
					if !ok {
						mismatch("record", "missing")
						continue
					}
					var data map[string]any
					json.Unmarshal(record.Data, &data)
					if data["body_status"] == "pending" {
						mismatch("body_status", "hydration_pending")
						continue
					}
					if body, ok := data["body_blob"].(map[string]any); ok {
						digest, _ := body["digest"].(string)
						status, bytes, blobErr := blobBytes(ctx, w.Gateway, digest, record.ID)
						if blobErr != nil || status != 200 || fmt.Sprintf("%x", sha256.Sum256(bytes)) != digest {
							mismatch("body_blob", "fetch_or_integrity")
							continue
						}
						var content map[string]any
						if json.Unmarshal(bytes, &content) != nil {
							mismatch("body_blob", "decode")
							continue
						}
						for _, field := range []string{"plain_text", "html", "blocks"} {
							data[field] = content[field]
						}
					}
					if data["title"] != native["title"] {
						mismatch("title", "value_or_type")
					}
					if entry.collection == "notes.note" {
						for _, field := range []string{"plain_text", "locked", "content_status"} {
							if data[field] != native[field] {
								if field == "content_status" {
									for label, value := range map[string]any{"production": data[field], "truth": native[field], "body": data["body_status"]} {
										status, _ := value.(string)
										switch status {
										case "complete", "partial", "offloaded", "undecoded", "locked", "pending", "external", "unavailable", "":
											mismatch("content_status."+label, fmt.Sprintf("%T_%s", value, status))
										default:
											mismatch("content_status."+label, "unknown_value")
										}
									}
								}
								mismatch(field, "value_or_type")
							}
						}
						if s.ID == "NOTE-DECODE-FULL" && data["content_status"] == "complete" {
							counts, countErr := notesProjectionCounts(data)
							truthCounts, ok := native["structure_counts"].(map[string]any)
							if countErr != nil || !ok {
								mismatch("structure_counts", "decode_or_missing")
								continue
							}
							for name, count := range counts {
								if !notesIntegerEqual(truthCounts[name], int64(count)) {
									kind := "count"
									if _, exact := notesExactInteger(truthCounts[name]); !exact {
										kind = "not_exact_integer"
									}
									mismatch("structure_counts."+name, kind)
								}
							}
						}
						if data["folder_id"] != aliases[aliasKey(contracts.Notes, "notes.folder", native["folder_id"].(string))] {
							mismatch("folder_id", "value_or_type")
						}
					}
				}
				return len(reasons) == 0
			})
			add(entry.key+"_equal_independent_truth", matches == nil)
			if matches != nil {
				details := make([]string, 0, len(reasons))
				for reason := range reasons {
					details = append(details, reason)
				}
				sort.Strings(details)
				e.Assertions[len(e.Assertions)-1].Detail = strings.Join(details, ", ")
			}
		}
		return
	}

	cleanupRun := contracts.NewID("notes")
	tag := "[ab-test " + cleanupRun + "]"
	title := tag + " note"
	endpoint := w.Gateway
	if level == "replica" {
		endpoint = w.Replicas[0]
		e.Hosts = append(e.Hosts, "ubuntu")
	}
	client := scripted.Client{Endpoint: endpoint, Protocol: "rest"}
	find := func(collection, title string) (contracts.Record, bool) {
		records, err := w.Records(endpoint)
		if err != nil {
			return contracts.Record{}, false
		}
		for _, r := range records {
			if r.Collection == collection && recordTitle(r) == title && !r.Deleted {
				var fields map[string]any
				json.Unmarshal(r.Data, &fields)
				if collection == "notes.note" && fields["recently_deleted"] == true {
					continue
				}
				return r, true
			}
		}
		return contracts.Record{}, false
	}
	var owned *runCleanup
	ownedFolders := map[string]string{}
	var fixtureID, fixtureFolderID string
	observeCleanup := func() ([]cleanupItem, error) {
		response, err := w.RecoverTruth("observe", map[string]any{"source": "notes", "tag": "", "active_only": true})
		if err != nil {
			return nil, err
		}
		rows, ok := response["records"].([]any)
		if !ok || response["status"] != "verified" {
			return nil, fmt.Errorf("notes cleanup truth unavailable")
		}
		items := make([]cleanupItem, 0, len(rows))
		for _, raw := range rows {
			row, ok := raw.(map[string]any)
			if !ok {
				return nil, fmt.Errorf("invalid notes cleanup identity")
			}
			id, _ := row["id"].(string)
			folder, _ := row["folder_id"].(string)
			title, _ := row["title"].(string)
			itemTag := ""
			if strings.HasPrefix(title, tag) {
				itemTag = tag
			}
			items = append(items, cleanupItem{ID: id, Container: folder, Tag: itemTag})
		}
		return items, nil
	}
	if level != "sim" && s.ID == "NOTE-CRUD" {
		before, problem := observeCleanup()
		if problem != nil {
			e.Status = "limited"
			e.Assertions = append(e.Assertions, Assertion{Name: "cleanup_baseline", Status: "limited", Detail: "needs Notes Automation grant for AppleTruth"})
			return
		}
		owned, err = newRunCleanup(cleanupRun, before)
		if err != nil {
			add("cleanup_baseline", false)
			return
		}
		owned.settle = 30 * time.Second
		owned.containers = ownedFolders
		owned.path = filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "run-cleanup", cleanupRun+".json")
		if owned.persist() != nil {
			add("cleanup_ledger", false)
			return
		}
		defer func() {
			proof, problem := owned.finish(observeCleanup, func(item cleanupItem) error {
				err := notesCleanupTruth(w.RecoverTruth, 30*time.Second, map[string]any{"source": "notes", "tag": tag, "action": "cleanup_owned", "id": item.ID, "folder_id": item.Container})
				return err
			})
			if e.Metrics == nil {
				e.Metrics = map[string]any{}
			}
			e.Metrics["cleanup"] = proof
			folderErr := notesCleanupFolders(w.RecoverTruth, tag, ownedFolders)
			for range ownedFolders {
				add("run_created_folder_cleaned", folderErr == nil)
			}
			add("owned_cleanup_counts_match", problem == nil)
		}()
	}
	fixture := func(action string, extra map[string]any) (map[string]any, error) {
		extra["source"] = "notes"
		extra["tag"] = tag
		extra["action"] = action
		if level != "sim" {
			extra["folder_title"] = "Bridge Test " + tag
			result, problem := w.Truth("fixture", extra)
			if problem == nil && action == "create" {
				fixtureID, _ = result["id"].(string)
				fixtureFolderID, _ = result["folder_id"].(string)
				if result["folder_created"] == true {
					ownedFolders[fixtureFolderID] = "Bridge Test " + tag
				}
			}
			if problem == nil && owned != nil && action == "create" {
				id, _ := result["id"].(string)
				folder, _ := result["folder_id"].(string)
				problem = owned.capture(cleanupItem{ID: id, Tag: tag, Container: folder})
				if result["folder_created"] == true {
					ownedFolders[folder] = "Bridge Test " + tag
					problem = owned.persist()
				}
			}
			return result, problem
		}
		if action == "create" {
			text := title + "\nsynthetic body\n"
			editable := "plain"
			if extra["html"] != nil {
				editable = "none"
			}
			data := map[string]any{"title": title, "plain_text": text, "folder_id": map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": "note-folder"}}, "locked": false, "editable": editable, "content_status": "complete", "attachments": []any{}}
			err := w.ControlAction("/external-change", map[string]any{"key": "external-note", "collection": "notes.note", "data": data})
			return map[string]any{"id": "external-note"}, err
		}
		return nil, nil
	}
	if level != "sim" && owned == nil {
		defer func() {
			if fixtureID != "" {
				err := notesCleanupTruth(w.RecoverTruth, 30*time.Second, map[string]any{"source": "notes", "tag": tag, "action": "cleanup_owned", "id": fixtureID, "folder_id": fixtureFolderID})
				add("tagged_notes_cleaned", err == nil)
			}
			problem := notesCleanupFolders(w.RecoverTruth, tag, ownedFolders)
			for range ownedFolders {
				add("run_created_folder_cleaned", problem == nil)
			}
		}()
	} else if level == "sim" {
		err = w.ControlAction("/external-change", map[string]any{"key": "note-folder", "collection": "notes.folder", "data": map[string]any{"title": "Bridge Test", "account": "sim"}})
		if err != nil {
			add("folder_fixture", false)
			return
		}
	}
	if s.ID == "NOTE-RICH-REJECT" {
		_, err = fixture("create", map[string]any{"title": title, "html": "<div>" + title + "</div><table><tr><td>synthetic</td></tr></table>"})
	} else {
		_, err = fixture("create", map[string]any{"title": title})
	}
	if err != nil {
		add("native_fixture", false)
		return
	}
	var note contracts.Record
	err = w.Await(30*time.Second, func() bool {
		var ok bool
		note, ok = find("notes.note", title)
		return ok && (level == "sim" || s.ID != "NOTE-RICH-REJECT" || notesContentReady(note, false))
	})
	add("external_note_imported", err == nil)
	if err != nil {
		if s.ID == "NOTE-EXTERNAL" {
			notesImportDiagnostics(w, &e, title)
			e.Metrics["notes_import_context_expired"] = ctx.Err() != nil
		}
		return
	}
	if s.ID == "NOTE-IMPORT" && level == "sim" {
		lockedData := map[string]any{"title": tag + " locked", "plain_text": "", "locked": true, "editable": "none", "content_status": "locked", "attachments": []any{}}
		err = w.ControlAction("/external-change", map[string]any{"key": "locked-note", "collection": "notes.note", "data": lockedData})
		add("locked_title_only", err == nil && w.Await(5*time.Second, func() bool {
			r, ok := find("notes.note", tag+" locked")
			var d map[string]any
			json.Unmarshal(r.Data, &d)
			return ok && d["plain_text"] == "" && d["content_status"] == "locked"
		}) == nil)
	}
	if s.ID == "NOTE-IMPORT" {
		var body map[string]any
		json.Unmarshal(note.Data, &body)
		add("folder_reference", body["folder_id"] != "" && body["folder_id"] != nil)
		if level != "sim" {
			truth, err := w.Truth("observe", map[string]any{"source": "notes", "tag": tag})
			rows, _ := truth["records"].([]any)
			add("independent_applescript_body", err == nil && len(rows) == 1 && strings.TrimRight(rows[0].(map[string]any)["plain_text"].(string), "\n") == strings.TrimRight(body["plain_text"].(string), "\n"))
		} else {
			add("plain_body", body["plain_text"] == title+"\nsynthetic body\n")
		}
		return
	}
	if s.ID == "NOTE-RICH-REJECT" {
		args, _ := json.Marshal(map[string]any{"id": note.ID, "plain_text": title + "\nreplacement"})
		var out contracts.CommandRecord
		err := client.Call(ctx, "notes.update", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: note.Version, WaitMS: 15000}, &out)
		add("rich_rejected_before_dispatch", errors.Is(err, contracts.ErrUnsupported) || (out.Error != nil && out.Error.Code == "unsupported"))
		problem := out.Error
		if problem == nil {
			errors.As(err, &problem)
		}
		add("rich_rejection_explains_recovery", problem != nil && strings.Contains(problem.Message, "Notes") && strings.Contains(problem.Message, "plain-text"))
		if level == "sim" {
			effects, _ := w.Effects()
			add("no_dispatch_effect", len(effects) == 0)
		}
		return
	}
	if s.ID == "NOTE-EXTERNAL" {
		err := measureNotesExternal(ctx, w, level, tag, note.ID, &e)
		if err != nil {
			fmt.Fprintln(os.Stderr, "notes visibility:", err)
		}
		add("external_edits_converge", err == nil)
		return
	}
	truthMatches := func(record contracts.Record) bool {
		var data map[string]any
		json.Unmarshal(record.Data, &data)
		expected, _ := data["plain_text"].(string)
		if level == "sim" {
			observations, err := w.Inspect()
			if err != nil {
				return false
			}
			for _, o := range observations {
				if o.Collection == "notes.note" && o.Kind == "upsert" {
					var native map[string]any
					json.Unmarshal(o.Data, &native)
					if native["title"] == data["title"] {
						return native["plain_text"] == expected
					}
				}
			}
			return false
		}
		response, err := w.Truth("observe", map[string]any{"source": "notes", "tag": tag})
		if err != nil {
			return false
		}
		rows, _ := response["records"].([]any)
		for _, row := range rows {
			native := row.(map[string]any)
			if native["title"] == data["title"] {
				text, _ := native["plain_text"].(string)
				return strings.TrimRight(text, "\n") == strings.TrimRight(expected, "\n")
			}
		}
		return false
	}
	if s.ID == "NOTE-CRUD" {
		for _, protocol := range []string{"rest", "mcp"} {
			client.Protocol = protocol
			var data struct {
				FolderID string `json:"folder_id"`
			}
			json.Unmarshal(note.Data, &data)
			before, _ := w.Records(endpoint)
			knownFolders := map[string]bool{}
			for _, r := range before {
				if r.Collection == "notes.folder" {
					knownFolders[r.ID] = true
				}
			}
			folderTitle := "Bridge Test " + tag + " " + protocol
			folderArgs, _ := json.Marshal(map[string]any{"title": folderTitle})
			var folderCommand contracts.CommandRecord
			err = client.Call(ctx, "notes.create_folder", contracts.Command{ID: contracts.NewCommandID(), Args: folderArgs, WaitMS: 15000}, &folderCommand)
			add(protocol+"_create_folder_observed", err == nil && folderCommand.State == "observed")
			e.Assertions[len(e.Assertions)-1].Detail = notesCommandDetail(err, folderCommand)
			if err != nil || folderCommand.State != "observed" {
				return
			}
			if owned != nil {
				identity, problem := w.Truth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "folder_identity", "title": folderTitle})
				id, _ := identity["id"].(string)
				add("created_folder_identity_captured", problem == nil && id != "")
				if problem != nil || id == "" {
					return
				}
				ownedFolders[id] = folderTitle
				if owned.persist() != nil {
					add("cleanup_ledger", false)
					return
				}
			}
			var createdFolder string
			err = w.Await(5*time.Second, func() bool {
				rows, _ := w.Records(endpoint)
				for _, r := range rows {
					if r.Collection == "notes.folder" && !knownFolders[r.ID] && !r.Deleted && recordTitle(r) == folderTitle {
						createdFolder = r.ID
						return true
					}
				}
				return false
			})
			if err != nil {
				add("created_folder_visible", false)
				return
			}
			data.FolderID = createdFolder
			if level != "sim" && owned == nil {
				var folderEvidence struct {
					NativeKey string `json:"native_key"`
				}
				json.Unmarshal(folderCommand.Evidence, &folderEvidence)
				defer func(key string) {
					_, cleanupErr := w.RecoverTruth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "cleanup_created_folder", "created_by_run": true, "native_key": key, "title": folderTitle})
					add("run_created_folder_cleaned", cleanupErr == nil)
				}(folderEvidence.NativeKey)
			}
			newTitle := tag + " " + protocol
			args, _ := json.Marshal(map[string]any{"title": newTitle, "folder_id": data.FolderID, "plain_text": "body"})
			var command contracts.CommandRecord
			err = client.Call(ctx, "notes.create", contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &command)
			add(protocol+"_create_observed", err == nil && command.State == "observed")
			e.Assertions[len(e.Assertions)-1].Detail = notesCommandDetail(err, command)
			if err != nil || command.State != "observed" {
				return
			}
			if owned != nil {
				var createdEvidence struct {
					NativeKey string `json:"native_key"`
				}
				json.Unmarshal(command.Evidence, &createdEvidence)
				rows, problem := w.Truth("observe", map[string]any{"source": "notes", "tag": tag})
				if problem != nil {
					add("created_identity_captured", false)
					return
				}
				captured := false
				for _, raw := range rows["records"].([]any) {
					row := raw.(map[string]any)
					if row["title"] == newTitle {
						id, _ := row["id"].(string)
						folder, _ := row["folder_id"].(string)
						captured = owned.capture(cleanupItem{ID: id, Tag: tag, Container: folder}) == nil
						ownedFolders[folder] = folderTitle
					}
				}
				add("created_identity_captured", captured)
				if !captured {
					return
				}
			}
			var created contracts.Record
			err = w.Await(30*time.Second, func() bool {
				var ok bool
				created, ok = find("notes.note", newTitle)
				return ok && (level == "sim" || notesContentReady(created, true))
			})
			if err != nil {
				add("created_visible", false)
				return
			}
			add(protocol+"_create_matches_truth", truthMatches(created))
			if level == "sim" {
				var evidence struct {
					NativeKey string `json:"native_key"`
				}
				json.Unmarshal(command.Evidence, &evidence)
				observations, inspectErr := w.Inspect()
				var metadata map[string]any
				for _, observation := range observations {
					if observation.NativeKey == evidence.NativeKey {
						json.Unmarshal(observation.Data, &metadata)
					}
				}
				if inspectErr != nil || metadata == nil {
					add(protocol+"_hydration_fixture", false)
					return
				}
				metadata["native_fingerprint"] = "ab-test-content-fingerprint"
				metadata["body_status"], metadata["content_status"], metadata["editable"] = "pending", "partial", "none"
				change := func() error {
					return w.ControlAction("/external-change", map[string]any{"key": evidence.NativeKey, "collection": "notes.note", "data": metadata})
				}
				previousVersion := created.Version
				if change() != nil || w.Await(5*time.Second, func() bool {
					current, ok := find("notes.note", newTitle)
					if ok && current.Version > previousVersion {
						created = current
						return true
					}
					return false
				}) != nil {
					add(protocol+"_metadata_visible", false)
					return
				}
				pendingID := contracts.NewCommandID()
				pendingArgs, _ := json.Marshal(map[string]any{"id": created.ID, "mode": "append", "plain_text": "\npending"})
				var pending contracts.CommandRecord
				pendingErr := client.Call(ctx, "notes.update", contracts.Command{ID: pendingID, Args: pendingArgs, ExpectedVersion: created.Version, WaitMS: 15000}, &pending)
				add(protocol+"_pending_content_rejected", errors.Is(pendingErr, contracts.ErrUnsupported) || pending.Error != nil && pending.Error.Code == "unsupported")
				pendingEffects, pendingEffectErr := w.Effects()
				add(protocol+"_pending_content_no_effect", pendingEffectErr == nil && pendingEffects[pendingID] == 0)
				metadata["content_status"], metadata["editable"] = "complete", "plain"
				delete(metadata, "body_status")
				err = change()
				add(protocol+"_hydration_advances_revision", err == nil && w.Await(5*time.Second, func() bool {
					current, ok := find("notes.note", newTitle)
					return ok && current.Version > created.Version
				}) == nil)
			}
			staleVersion := created.Version

			for _, mode := range []string{"append", "replace"} {
				replacement := "\nappended"
				if mode == "replace" {
					replacement = newTitle + "\nreplaced"
				}
				args, _ = json.Marshal(map[string]any{"id": created.ID, "mode": mode, "plain_text": replacement})
				err = client.Call(ctx, "notes.update", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: created.Version, WaitMS: 15000}, &command)
				add(protocol+"_"+mode+"_observed", err == nil && command.State == "observed")
				if err != nil || command.State != "observed" {
					e.Assertions[len(e.Assertions)-1].Detail = notesCommandDetail(err, command)
					return
				}
				err = w.Await(30*time.Second, func() bool {
					current, ok := find("notes.note", newTitle)
					if ok && current.Version > created.Version && (level == "sim" || notesContentReady(current, true)) {
						created = current
						return true
					}
					return false
				})
				add(protocol+"_"+mode+"_visible", err == nil)
				add(protocol+"_"+mode+"_matches_truth", truthMatches(created))
				var updated map[string]any
				json.Unmarshal(created.Data, &updated)
				actual, _ := updated["plain_text"].(string)
				add(protocol+"_"+mode+"_content", strings.Contains(actual, strings.TrimSpace(replacement)))
				if level == "sim" && mode == "append" {
					staleID := contracts.NewCommandID()
					var rejected contracts.CommandRecord
					staleErr := client.Call(ctx, "notes.update", contracts.Command{ID: staleID, Args: args, ExpectedVersion: staleVersion, WaitMS: 15000}, &rejected)
					add(protocol+"_content_change_rejects_stale_version", errors.Is(staleErr, contracts.ErrConflict) || rejected.Error != nil && rejected.Error.Code == "conflict")
					effects, effectErr := w.Effects()
					add(protocol+"_stale_version_no_effect", effectErr == nil && effects[staleID] == 0)
				}

			}
			replicated := func(deleted bool) bool {
				for _, target := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
					rows, problem := w.Records(target)
					if problem != nil {
						return false
					}
					present := false
					for _, row := range rows {
						if row.ID == created.ID && !row.Deleted {
							if !deleted && row.Version < created.Version {
								return false
							}
							present = true
						}
					}
					if present == deleted {
						return false
					}
				}
				return true
			}
			err = w.Await(30*time.Second, func() bool { return replicated(false) })
			add(protocol+"_before_delete_replicated", err == nil)
			if err != nil {
				return
			}
			args, _ = json.Marshal(map[string]any{"id": created.ID})
			command = contracts.CommandRecord{}
			deleteID := contracts.NewCommandID()
			err = client.Call(ctx, "notes.delete", contracts.Command{ID: deleteID, Args: args, ExpectedVersion: created.Version, WaitMS: 15000}, &command)
			deleteCode := ""
			if err != nil {
				var problem *contracts.Error
				if errors.As(err, &problem) {
					deleteCode = problem.Code
				} else {
					deleteCode = "transport"
				}
			}
			if command.Error != nil {
				deleteCode = command.Error.Code
			}
			e.Assertions = append(e.Assertions, Assertion{Name: protocol + "_delete_observed", Passed: err == nil && command.State == "observed", Detail: string(command.State) + ":" + deleteCode})
			add(protocol+"_delete_replicated", w.Await(30*time.Second, func() bool { return replicated(true) }) == nil)
			_, fixturePresent := find("notes.note", title)
			add(protocol+"_delete_preserves_other_note", fixturePresent)
			if level == "sim" {
				effects, problem := w.Effects()
				add(protocol+"_delete_applied_once", problem == nil && effects[deleteID] == 1)
			}
		}
	}
	return
}

func notesContentReady(record contracts.Record, plain bool) bool {
	var data struct {
		Editable      string `json:"editable"`
		BodyStatus    string `json:"body_status"`
		ContentStatus string `json:"content_status"`
	}
	return json.Unmarshal(record.Data, &data) == nil && data.BodyStatus != "pending" && data.ContentStatus == "complete" && (!plain || data.Editable == "plain")
}

func notesCommandDetail(err error, command contracts.CommandRecord) string {
	code := ""
	var problem *contracts.Error
	if errors.As(err, &problem) {
		code = problem.Code
	} else if err != nil {
		code = "transport"
	}
	if command.Error != nil {
		code = command.Error.Code
	}
	var evidence struct {
		Reason      string `json:"reason"`
		NativeError string `json:"native_error_code"`
	}
	json.Unmarshal(command.Evidence, &evidence)
	return string(command.State) + ":" + code + ":" + evidence.Reason + ":" + evidence.NativeError
}

func notesNativeBuildLock(path string) func() {
	previous, present := os.LookupEnv("APPLE_BRIDGE_NATIVE_LOCK")
	if previous != "" {
		return func() {}
	}
	target, err := filepath.EvalSymlinks(path)
	if err != nil {
		return func() {}
	}
	lane := filepath.Base(target)
	if lane != "ab-swift-build-1" && lane != "ab-swift-build-2" {
		return func() {}
	}
	os.Setenv("APPLE_BRIDGE_NATIVE_LOCK", "/tmp/ab-native-lane-"+strings.TrimPrefix(lane, "ab-swift-build-")+".lock")
	return func() {
		if present {
			os.Setenv("APPLE_BRIDGE_NATIVE_LOCK", previous)
		} else {
			os.Unsetenv("APPLE_BRIDGE_NATIVE_LOCK")
		}
	}
}
