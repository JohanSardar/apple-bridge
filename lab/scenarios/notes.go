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
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
)

func runNotes(ctx context.Context, s Scenario, level string) (e Evidence) {
	if s.ID == "NOTE-LIST" {
		return runNotesList(ctx, s, level)
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
	if (s.ID == "NOTE-IMPORT" || s.ID == "NOTE-DECODE-FULL") && level == "real" {
		response, truthErr := w.Truth("observe", map[string]any{"source": "notes", "sqlite": true})
		if truthErr != nil {
			add("independent_sqlite_copy", false)
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
			matches := w.Await(10*time.Second, func() bool {
				aliases, _ = gatewayAliases(w)
				records, readErr := w.Records(w.Gateway)
				if readErr != nil {
					return false
				}
				actual := map[string]contracts.Record{}
				for _, record := range records {
					if record.Collection == entry.collection && !record.Deleted {
						actual[record.ID] = record
					}
				}
				if len(actual) != len(truthRows) {
					return false
				}
				for _, value := range truthRows {
					native := value.(map[string]any)
					id := aliases[aliasKey(contracts.Notes, entry.collection, native["id"].(string))]
					record, ok := actual[id]
					if !ok {
						return false
					}
					var data map[string]any
					json.Unmarshal(record.Data, &data)
					if body, ok := data["body_blob"].(map[string]any); ok {
						digest, _ := body["digest"].(string)
						status, bytes, blobErr := blobBytes(ctx, w.Gateway, digest, record.ID)
						if blobErr != nil || status != 200 || fmt.Sprintf("%x", sha256.Sum256(bytes)) != digest {
							return false
						}
						var content map[string]any
						if json.Unmarshal(bytes, &content) != nil {
							return false
						}
						for _, field := range []string{"plain_text", "html", "blocks"} {
							data[field] = content[field]
						}
					}
					if data["title"] != native["title"] {
						return false
					}
					if entry.collection == "notes.note" {
						for _, field := range []string{"plain_text", "locked", "content_status"} {
							if data[field] != native[field] {
								return false
							}
						}
						if s.ID == "NOTE-DECODE-FULL" && data["content_status"] == "complete" {
							counts, countErr := notesProjectionCounts(data)
							truthCounts, ok := native["structure_counts"].(map[string]any)
							if countErr != nil || !ok {
								return false
							}
							for name, count := range counts {
								if truthCounts[name] != float64(count) {
									return false
								}
							}
						}
						if data["folder_id"] != aliases[aliasKey(contracts.Notes, "notes.folder", native["folder_id"].(string))] {
							return false
						}
					}
				}
				return true
			})
			add(entry.key+"_equal_independent_truth", matches == nil)
		}
		return
	}

	tag := "[ab-test " + contracts.NewID("notes") + "]"
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
	fixture := func(action string, extra map[string]any) (map[string]any, error) {
		extra["source"] = "notes"
		extra["tag"] = tag
		extra["action"] = action
		if level != "sim" {
			return w.Truth("fixture", extra)
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
	if level != "sim" {
		defer func() {
			_, err := w.RecoverTruth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "cleanup"})
			add("tagged_notes_cleaned", err == nil)
		}()
	} else {
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
	err = w.Await(15*time.Second, func() bool { var ok bool; note, ok = find("notes.note", title); return ok })
	add("external_note_imported", err == nil)
	if err != nil {
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
		if level == "sim" {
			effects, _ := w.Effects()
			add("no_dispatch_effect", len(effects) == 0)
		}
		return
	}
	if s.ID == "NOTE-EXTERNAL" {
		samples := []float64{}
		for i := 0; i < 5; i++ {
			replacement := fmt.Sprintf("%s edit-%d", tag, i)
			effectNS := world.UptimeNS()
			if level == "sim" {
				data := map[string]any{"title": replacement, "plain_text": replacement + "\nbody\n", "locked": false, "editable": "plain", "folder_id": map[string]any{"$ref": map[string]string{"collection": "notes.folder", "native_key": "note-folder"}}}
				err = w.ControlAction("/external-change", map[string]any{"key": "external-note", "collection": "notes.note", "data": data})
			} else {
				response, fixtureErr := w.Truth("observe", map[string]any{"source": "notes", "tag": tag})
				if fixtureErr != nil {
					add("truth_before_edit", false)
					return
				}
				rows, _ := response["records"].([]any)
				if len(rows) != 1 {
					add("truth_identity", false)
					return
				}
				response, err = w.Truth("fixture", map[string]any{"source": "notes", "tag": tag, "action": "modify", "id": rows[0].(map[string]any)["id"], "plain_text": replacement + "\nbody"})
				if err == nil {
					if number, ok := response["effect_uptime_ns"].(json.Number); ok {
						effectNS, _ = number.Int64()
					}
				}
			}
			if level == "sim" {
				effectNS = world.UptimeNS()
			}
			if err != nil {
				add("native_edit", false)
				return
			}
			err = w.Await(15*time.Second, func() bool { _, ok := find("notes.note", replacement); return ok })

			if err != nil {
				add("external_edit_visible", false)
				return
			}
			samples = append(samples, float64(world.UptimeNS()-effectNS)/float64(time.Millisecond))
		}
		e.Latency["native_save_to_copy_visible_ms"] = stats(samples, 0)
		add("external_edits_converge", true)
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
			if err != nil || folderCommand.State != "observed" {
				return
			}
			var createdFolder string
			err = w.Await(5*time.Second, func() bool {
				rows, _ := w.Records(endpoint)
				for _, r := range rows {
					if r.Collection == "notes.folder" && !knownFolders[r.ID] && !r.Deleted {
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
			if level != "sim" {
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
			if err != nil || command.State != "observed" {
				return
			}
			var created contracts.Record
			err = w.Await(10*time.Second, func() bool { var ok bool; created, ok = find("notes.note", newTitle); return ok })
			if err != nil {
				add("created_visible", false)
				return
			}
			add(protocol+"_create_matches_truth", truthMatches(created))
			for _, mode := range []string{"append", "replace"} {
				replacement := "\nappended"
				if mode == "replace" {
					replacement = newTitle + "\nreplaced"
				}
				args, _ = json.Marshal(map[string]any{"id": created.ID, "mode": mode, "plain_text": replacement})
				err = client.Call(ctx, "notes.update", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: created.Version, WaitMS: 15000}, &command)
				add(protocol+"_"+mode+"_observed", err == nil && command.State == "observed")
				if err != nil || command.State != "observed" {
					return
				}
				err = w.Await(5*time.Second, func() bool {
					current, ok := find("notes.note", newTitle)
					if ok && current.Version > created.Version {
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
			}
			args, _ = json.Marshal(map[string]any{"id": created.ID})
			command = contracts.CommandRecord{}
			err = client.Call(ctx, "notes.delete", contracts.Command{ID: contracts.NewCommandID(), Args: args, ExpectedVersion: created.Version, WaitMS: 15000}, &command)
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
			add(protocol+"_delete_replicated", w.Await(5*time.Second, func() bool { _, ok := find("notes.note", newTitle); return !ok }) == nil)
		}
	}
	return
}
