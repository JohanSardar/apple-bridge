package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/runtime"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strconv"
	"strings"
	"time"
)

func runVisionQueries(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	var w *world.World
	var err error
	if level == "real" {
		w, err = world.NewReal(ctx, level, 0)
	} else {
		w, err = world.New(ctx, level, 1, 0)
	}
	add("world_started", err == nil)
	if err != nil {
		fmt.Fprintln(os.Stderr, "vision world startup:", err)
		return
	}
	defer w.Close()
	endpoint := w.Gateway
	endpoint.Token, err = w.ScopedToken(endpoint, "vision:read,pdf:read,system:read")
	add("query_scopes_granted", err == nil)
	if err != nil {
		return
	}
	client := scripted.Client{Endpoint: endpoint}
	{
		readyErr := w.Await(15*time.Second, func() bool {
			var version contracts.QueryResult
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &version) == nil
		})
		add("helper_ready", readyErr == nil)
		if readyErr != nil {
			return
		}
	}
	if level == "real" {
		readinessErr := w.Await(15*time.Second, func() bool {
			report, problem := runtime.Doctor(ctx, w.Gateway.Home)
			return problem == nil && report.DocumentChecks["vision"] == "ready" && report.DocumentChecks["pdf"] == "ready"
		})
		add("doctor_document_sources_ready", readinessErr == nil)
		if readinessErr != nil {
			return
		}
	}
	if s.ID == "VIS-IDLE" {
		pid, problem := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
		add("helper_pid_available", problem == nil)
		if problem != nil {
			return
		}
		cpu := func() float64 {
			output, problem := exec.CommandContext(ctx, "ps", "-o", "time=", "-p", strings.TrimSpace(string(pid))).Output()
			parts := strings.Split(strings.TrimSpace(string(output)), ":")
			if problem != nil || len(parts) != 2 {
				return -1
			}
			minutes, first := strconv.ParseFloat(parts[0], 64)
			seconds, second := strconv.ParseFloat(parts[1], 64)
			if first != nil || second != nil {
				return -1
			}
			return minutes*60 + seconds
		}
		before := cpu()
		started := time.Now()
		timer := time.NewTimer(5 * time.Minute)
		defer timer.Stop()
		select {
		case <-ctx.Done():
			add("idle_completed", false)
			return
		case <-timer.C:
		}
		after := cpu()
		elapsed := time.Since(started).Seconds()
		e.Metrics = map[string]any{"helper_cpu_percent": 100 * (after - before) / elapsed, "idle_seconds": elapsed}
		add("helper_cpu_under_two_percent", before >= 0 && after >= before && (after-before)/elapsed < 0.02)
		return
	}
	if s.ID == "PDF-ANNOTATE" {
		data, readErr := os.ReadFile(filepath.Join("lab", "fixtures", "vision", "fixture.pdf"))
		add("fixture_read", readErr == nil)
		args, _ := json.Marshal(map[string]any{"data_base64": base64.StdEncoding.EncodeToString(data), "annotations": []any{map[string]any{"page": 1, "type": "note", "text": "[ab-test synthetic]", "x": 10, "y": 10, "width": 20, "height": 20}, map[string]any{"page": 1, "type": "highlight", "text": "", "x": 40, "y": 40, "width": 50, "height": 15}}})
		endpoint.Token, err = w.ScopedToken(endpoint, "pdf:read,pdf:write")
		add("annotation_scopes", err == nil)
		client.Endpoint = endpoint
		badArgs, _ := json.Marshal(map[string]any{"data_base64": base64.StdEncoding.EncodeToString(data), "annotations": []any{}})
		var rejected contracts.CommandRecord
		badErr := client.Call(ctx, "pdf.annotate", contracts.Command{ID: contracts.NewCommandID(), Args: badArgs, WaitMS: 15000}, &rejected)
		var invalid *contracts.Error
		add("empty_annotations_rejected", errors.As(badErr, &invalid) && invalid.Code == "invalid")
		var result contracts.CommandRecord
		commandID := contracts.NewCommandID()
		err = client.Call(ctx, "pdf.annotate", contracts.Command{ID: commandID, Args: args, WaitMS: 15000}, &result)
		e.Metrics = map[string]any{"observed_record_count": len(result.ObservedRecordIDs)}
		var annotationProblem *contracts.Error
		if errors.As(err, &annotationProblem) {
			e.Metrics["annotation_error_code"] = annotationProblem.Code
		}
		if result.Error != nil {
			e.Metrics["command_error_code"] = result.Error.Code
		}
		add("annotation_observed", err == nil && result.State == "observed" && len(result.ObservedRecordIDs) == 1)
		if err != nil || len(result.ObservedRecordIDs) != 1 {
			return
		}
		var records contracts.ReadResult
		err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "pdf.artifact", ID: result.ObservedRecordIDs[0]}, &records)
		add("artifact_metadata", err == nil && len(records.Records) == 1)
		if err != nil || len(records.Records) != 1 {
			return
		}
		var metadata struct {
			Blob  contracts.BlobReference `json:"blob"`
			Count int                     `json:"annotation_count"`
		}
		err = json.Unmarshal(records.Records[0].Data, &metadata)
		add("annotation_count", err == nil && metadata.Count == 2)
		var output []byte
		err = w.Await(10*time.Second, func() bool {
			req, _ := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/blobs/"+metadata.Blob.Digest+"?record="+result.ObservedRecordIDs[0], nil)
			req.Host = endpoint.Host
			req.Header.Set("Authorization", "Bearer "+endpoint.Token)
			response, problem := http.DefaultClient.Do(req)
			if problem != nil {
				return false
			}
			defer response.Body.Close()
			output, problem = io.ReadAll(io.LimitReader(response.Body, 16<<20))
			return problem == nil && response.StatusCode == 200
		})
		sum := sha256.Sum256(output)
		add("new_pdf_blob_digest", err == nil && hex.EncodeToString(sum[:]) == metadata.Blob.Digest && !bytes.Equal(output, data))
		blobArgs, _ := json.Marshal(map[string]any{"blob": map[string]string{"collection": "pdf.artifact", "record_id": result.ObservedRecordIDs[0], "digest": metadata.Blob.Digest}})
		var extracted contracts.QueryResult
		blobErr := client.Call(ctx, "pdf.text", contracts.Query{Args: blobArgs, DeadlineMS: 10000}, &extracted)
		var content struct {
			Pages []struct {
				Text string `json:"text"`
			} `json:"pages"`
		}
		contentErr := json.Unmarshal(extracted.ResultJSON, &content)
		add("artifact_blob_input_preserves_document_text", blobErr == nil && contentErr == nil && len(content.Pages) == 1 && strings.TrimSpace(content.Pages[0].Text) == "Bridge fixture")
		add("annotation_page_geometry_preserved", bytes.Contains(output, []byte("/MediaBox [0 0 600 800]")))
		add("output_contains_note_and_highlight", bytes.Contains(output, []byte("/Subtype /Text")) && bytes.Contains(output, []byte("/Subtype /Highlight")) && bytes.Contains(output, []byte("/Contents ([ab-test synthetic])")))
		return
	}
	op := "vision.ocr"
	if s.ID == "PDF-TEXT" {
		op = "pdf.text"
	}
	args, _ := json.Marshal(map[string]string{"data_base64": base64.StdEncoding.EncodeToString([]byte("invalid fixture"))})
	var out contracts.QueryResult
	err = client.Call(ctx, op, contracts.Query{Args: args, DeadlineMS: 10000}, &out)
	var problem *contracts.Error
	add("malformed_document_rejected", errors.As(err, &problem) && problem.Code == "invalid")
	files := []string{"en.png", "fr.png", "de.png"}
	texts := []string{"Bridge fixture", "Bonjour le monde", "Guten Morgen"}
	if op == "pdf.text" {
		files = []string{"fixture.pdf"}
		texts = []string{"Bridge fixture"}
	}
	for i, file := range files {
		data, readErr := os.ReadFile(filepath.Join("lab", "fixtures", "vision", file))
		add("generated_fixture_read", readErr == nil)
		if readErr != nil {
			return
		}
		args, _ = json.Marshal(map[string]string{"data_base64": base64.StdEncoding.EncodeToString(data)})
		err = client.Call(ctx, op, contracts.Query{Args: args, DeadlineMS: 30000}, &out)
		add("fixture_query_completed", err == nil)
		var content struct {
			Text  string `json:"text"`
			Pages []struct {
				Text string `json:"text"`
			} `json:"pages"`
		}
		contentErr := json.Unmarshal(out.ResultJSON, &content)
		if op == "pdf.text" && len(content.Pages) == 1 {
			content.Text = strings.TrimSpace(content.Pages[0].Text)
		}
		add("fixture_content_matched", contentErr == nil && content.Text == texts[i])
		path, pathErr := filepath.Abs(filepath.Join("lab", "fixtures", "vision", file))
		fileArgs, _ := json.Marshal(map[string]any{"file": path, "allow_file": true})
		var denied contracts.QueryResult
		deniedArgs, _ := json.Marshal(map[string]string{"file": path})
		deniedErr := client.Call(ctx, op, contracts.Query{Args: deniedArgs, DeadlineMS: 10000}, &denied)
		add("file_requires_opt_in", errors.As(deniedErr, &problem) && problem.Code == "invalid")
		blobArgs, _ := json.Marshal(map[string]any{"blob": map[string]string{"query_id": "qry_missing", "digest": strings.Repeat("0", 64)}})
		blobErr := client.Call(ctx, op, contracts.Query{Args: blobArgs, DeadlineMS: 10000}, &denied)
		add("unowned_blob_rejected", errors.As(blobErr, &problem) && problem.Code == "forbidden")
		var fileOut contracts.QueryResult
		fileErr := client.Call(ctx, op, contracts.Query{Args: fileArgs, DeadlineMS: 30000}, &fileOut)
		var inlineValue, fileValue any
		inlineDecode := json.Unmarshal(out.ResultJSON, &inlineValue)
		fileDecode := json.Unmarshal(fileOut.ResultJSON, &fileValue)
		add("file_input_matches_inline", pathErr == nil && fileErr == nil && inlineDecode == nil && fileDecode == nil && reflect.DeepEqual(inlineValue, fileValue))
		if op == "vision.ocr" {
			var result struct {
				Lines []struct {
					Box map[string]float64 `json:"box"`
				} `json:"lines"`
				Language string `json:"language"`
			}
			decodeErr := json.Unmarshal(out.ResultJSON, &result)
			add("ocr_shape", decodeErr == nil && len(result.Lines) > 0 && len(result.Lines[0].Box) == 4 && result.Language != "")
		}
		if op == "pdf.text" {
			for _, needle := range []string{"fixture", "missing"} {
				searchArgs, _ := json.Marshal(map[string]string{"data_base64": base64.StdEncoding.EncodeToString(data), "text": needle})
				err = client.Call(ctx, "pdf.search", contracts.Query{Args: searchArgs, DeadlineMS: 10000}, &out)
				var matches struct {
					Matches []struct {
						Page   int `json:"page"`
						Offset int `json:"offset"`
						Length int `json:"length"`
					} `json:"matches"`
				}
				decodeErr := json.Unmarshal(out.ResultJSON, &matches)
				valid := err == nil && decodeErr == nil
				if needle == "fixture" {
					valid = valid && len(matches.Matches) == 1
					if len(matches.Matches) == 1 {
						valid = valid && matches.Matches[0].Page == 1 && matches.Matches[0].Length == 7
					}
				} else {
					valid = valid && len(matches.Matches) == 0
				}
				add("pdf_search_match_count_and_page", valid)
			}
		}
	}
	return
}
