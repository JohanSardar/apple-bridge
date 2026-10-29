package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"io"
	"net/http"
	"strings"
	"time"
)

func runAudioQueries(ctx context.Context, s Scenario, level string) (e Evidence) {
	e = Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"air"}, Latency: map[string]Latency{}}
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
	endpoint.Token, err = w.ScopedToken(endpoint, "speech:read,speech:write,capture:write,ai:read,audio:read,system:read")
	add("scopes_granted", err == nil)
	if err != nil {
		return
	}
	client := scripted.Client{Endpoint: endpoint}
	{
		err = w.Await(15*time.Second, func() bool {
			var out contracts.QueryResult
			return client.Call(ctx, "helper.version", contracts.Query{Args: json.RawMessage(`{}`)}, &out) == nil
		})
		add("helper_ready", err == nil)
		if err != nil {
			return
		}
	}

	if s.ID == "SPEECH-SAY" || s.ID == "AUD-SIM" {
		for _, op := range []string{"speech.say", "capture.play_sound"} {
			args := json.RawMessage(`{"text":"Bridge fixture","volume":0}`)
			if op == "capture.play_sound" {
				args = json.RawMessage(`{"name":"Tink","volume":0}`)
			}
			var record contracts.CommandRecord
			err = client.Call(ctx, op, contracts.Command{ID: contracts.NewCommandID(), Args: args, WaitMS: 15000}, &record)
			add("playback_finished_callback", err == nil && record.State == "observed")
		}
		if level == "sim" {
			var record contracts.CommandRecord
			id := contracts.NewCommandID()
			err = client.Call(ctx, "speech.say", contracts.Command{ID: id, Args: json.RawMessage(`{"text":"[ab-test missing callback]","volume":0}`), WaitMS: 15000}, &record)
			statusErr := client.Call(ctx, "commands.status", contracts.CommandQuery{ID: id}, &record)
			add("missing_callback_is_indeterminate", statusErr == nil && record.State == "indeterminate")
		}
		if s.ID == "SPEECH-SAY" {
			return
		}
	}

	if s.ID == "AUD-SIM" {
		for _, op := range []string{"ai.local", "ai.rewrite"} {
			args := json.RawMessage(`{"prompt":"Bridge fixture"}`)
			if op == "ai.rewrite" {
				args = json.RawMessage(`{"text":"Bridge fixture","style":"formal"}`)
			}
			var output contracts.QueryResult
			callErr := client.Call(ctx, op, contracts.Query{Args: args, DeadlineMS: 1000}, &output)
			var value struct {
				Status string `json:"status"`
				Reason string `json:"reason"`
			}
			decodeErr := json.Unmarshal(output.ResultJSON, &value)
			add("sim_model_availability_explicit", callErr == nil && decodeErr == nil && value.Status == "unavailable" && value.Reason != "")
		}
	}
	var out contracts.QueryResult
	if s.ID == "AI-LOCAL" {
		err = client.Call(ctx, "ai.local", contracts.Query{Args: json.RawMessage(`{"prompt":"Return a JSON object with a single key named count and integer value 3.","schema":{"type":"object","properties":{"count":{"type":"integer"}},"required":["count"],"additionalProperties":false}}`), DeadlineMS: 60000}, &out)
		var result struct {
			Status string `json:"status"`
			Reason string `json:"reason"`
			Text   string `json:"text"`
		}
		decode := json.Unmarshal(out.ResultJSON, &result)
		valid := result.Status == "unavailable" && result.Reason != ""
		if result.Status == "available" {
			var fixed struct {
				Count int `json:"count"`
			}
			valid = contracts.DecodeInput([]byte(result.Text), &fixed) == nil && fixed.Count == 3
		}
		add("fixed_schema_or_explicit_unavailability", err == nil && decode == nil && valid)
		return
	}
	err = client.Call(ctx, "speech.synthesize", contracts.Query{Args: json.RawMessage(`{"text":"Bridge fixture"}`), DeadlineMS: 60000}, &out)
	var audio struct {
		Data  string `json:"data_base64"`
		Media string `json:"media_type"`
	}
	decode := json.Unmarshal(out.ResultJSON, &audio)
	add("synthesis_completed", err == nil && decode == nil && audio.Media == "audio/wav" && len(out.Blobs) == 1 && out.Blobs[0].Size > 44)

	if len(out.Blobs) == 1 {
		blob := out.Blobs[0]
		req, _ := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/blobs/"+blob.Digest+"?record="+out.ID, nil)
		req.Host = endpoint.Host
		req.Header.Set("Authorization", "Bearer "+endpoint.Token)
		response, readErr := http.DefaultClient.Do(req)
		valid := false
		if readErr == nil {
			data, _ := io.ReadAll(io.LimitReader(response.Body, 700001))
			response.Body.Close()
			sum := sha256.Sum256(data)
			valid = response.StatusCode == 200 && len(data) > 44 && int64(len(data)) == blob.Size && string(data[:4]) == "RIFF" && hex.EncodeToString(sum[:]) == blob.Digest
		}
		add("wav_blob_readback_digest", valid)
		if (s.ID == "AUD-TRANSCRIBE" || s.ID == "AUD-SIM") && valid {
			response, readErr = http.DefaultClient.Do(req)
			if readErr != nil {
				add("fixture_read", false)
				return
			}
			data, _ := io.ReadAll(io.LimitReader(response.Body, 700001))
			response.Body.Close()
			args, _ := json.Marshal(map[string]string{"data_base64": base64.StdEncoding.EncodeToString(data)})
			err = client.Call(ctx, "audio.transcribe", contracts.Query{Args: args, DeadlineMS: 60000}, &out)
			var result struct {
				Status   string `json:"status"`
				Reason   string `json:"reason"`
				Text     string `json:"text"`
				Language string `json:"language"`
				Segments []struct {
					Start float64 `json:"start"`
					End   float64 `json:"end"`
				} `json:"segments"`
			}
			decode := json.Unmarshal(out.ResultJSON, &result)
			valid := result.Status == "unavailable" && result.Reason == "Speech Recognition authorization for the Helper is not_determined"
			if result.Status == "available" {
				words := strings.FieldsFunc(strings.ToLower(result.Text), func(r rune) bool { return r < 'a' || r > 'z' })
				matched := 0
				for _, expected := range []string{"bridge", "fixture"} {
					for _, word := range words {
						if word == expected {
							matched++
							break
						}
					}
				}
				ratio := float64(matched) / 2
				e.Metrics = map[string]any{"word_overlap_ratio": ratio}
				valid = ratio >= 0.8 && result.Language != "" && len(result.Segments) > 0
				for _, segment := range result.Segments {
					valid = valid && segment.Start >= 0 && segment.End >= segment.Start
				}
			}
			add("on_device_transcript_or_authorization_limit", err == nil && decode == nil && valid)
		}
	}
	err = client.Call(ctx, "speech.synthesize", contracts.Query{Args: json.RawMessage(`{"text":""}`)}, &out)
	var problem *contracts.Error
	add("empty_text_rejected", errors.As(err, &problem) && problem.Code == "invalid")
	return
}
