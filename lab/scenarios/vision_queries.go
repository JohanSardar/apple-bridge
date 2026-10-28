package scenarios

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
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
	if level == "real" {
		readyErr := w.Await(15*time.Second, func() bool {
			var version contracts.QueryResult
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`), DeadlineMS: 1000}, &version) == nil
		})
		add("helper_ready", readyErr == nil)
		if readyErr != nil {
			return
		}
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
		fileArgs, _ := json.Marshal(map[string]string{"file": path})
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
