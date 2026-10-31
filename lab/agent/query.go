package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

const queryArgs = `{"from":{"lat":37.7955,"lon":-122.3937},"to":{"lat":37.8024,"lon":-122.4058},"mode":"driving"}`

func Query(ctx context.Context, level string, asJSON bool) error {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	transport := "replica"
	if level == "sim" {
		transport = "sim"
	}
	if err := world.Build(ctx, transport); err != nil {
		return err
	}
	w, err := world.New(ctx, transport, 1, 0, world.BlobOptions{Scope: "maps:read,reminders:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	var out outcome
	var transcript []byte
	var runErr, readErr error
	if level == "sim" {
		out, transcript, runErr = simulatedQuery(ctx, w)
	} else {
		driver, err := os.ReadFile("lab/agent/codex.py")
		if err != nil {
			return err
		}
		private := filepath.Join("lab/evidence/private", contracts.NewID("query"))
		runner := AgentRunner{Driver: driver, Private: private}
		prompt := `Use only apple_bridge MCP tools. What is the driving ETA and distance from San Francisco Ferry Building (latitude 37.7955, longitude -122.3937) to Coit Tower (latitude 37.8024, longitude -122.4058)? Call maps_eta with from/to lat/lon points and mode driving. This is a synthetic provider, not live traffic. Cite maps.eta and report the returned duration in milliseconds and distance in meters, preserving the numeric values and units. Label the result synthetic and use a single plain-text sentence.`
		out, runErr = runner.execute(ctx, w, Task{ID: "QUERY-AGENT", Scope: "maps:read"}, 1, prompt, []string{"maps_eta"})
		transcript, readErr = os.ReadFile(filepath.Join(private, "QUERY-AGENT-1", "transcript.jsonl"))
	}
	tools, answerOK := queryChecks(transcript, out.Answer)
	passed := runErr == nil && readErr == nil && out.Exit == 0 && out.Calls >= 1 && out.Forbidden == 0 && tools && answerOK
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	evidence := map[string]any{"at": time.Now().UTC(), "git_sha": strings.TrimSpace(string(sha)), "scenario": "QUERY-AGENT", "level": level, "passed": passed, "mcp_calls": out.Calls, "forbidden_calls": out.Forbidden, "tools_check": tools, "answer_check": answerOK}
	if !passed {
		evidence["failure"] = "agent_execution_or_answer"
	}
	levels := []string{level}
	if level == "real" {
		levels = append(levels, "agent")
	}
	var raw []byte
	for _, proofLevel := range levels {
		evidence["level"] = proofLevel
		raw, err = json.MarshalIndent(evidence, "", "  ")
		if err != nil {
			return err
		}
		directory := filepath.Join("lab/evidence", proofLevel)
		if err = os.MkdirAll(directory, 0700); err != nil {
			return err
		}
		if err = os.WriteFile(filepath.Join(directory, "QUERY-AGENT.json"), append(raw, '\n'), 0600); err != nil {
			return err
		}
	}
	evidence["level"] = level
	raw, err = json.MarshalIndent(evidence, "", "  ")
	if err != nil {
		return err
	}
	if asJSON {
		fmt.Println(string(raw))
	} else {
		fmt.Printf("QUERY-AGENT %s passed=%t tools=%t answer=%t mcp_calls=%d forbidden_calls=%d\n", level, passed, tools, answerOK, out.Calls, out.Forbidden)
	}
	if !passed {
		return fmt.Errorf("QUERY-AGENT failed: check the private agent transcript and confirm Codex on Ubuntu can reach the bridge")
	}
	return nil
}

type queryEvent struct {
	Type string `json:"type"`
	Item struct {
		Type      string          `json:"type"`
		Server    string          `json:"server"`
		Tool      string          `json:"tool"`
		Status    string          `json:"status"`
		Arguments contracts.Query `json:"arguments"`
		Error     json.RawMessage `json:"error"`
		Result    *struct {
			IsError                bool                  `json:"isError"`
			StructuredContent      contracts.QueryResult `json:"structuredContent"`
			CodexStructuredContent contracts.QueryResult `json:"structured_content"`
			Content                []struct {
				Text string `json:"text"`
			} `json:"content"`
		} `json:"result"`
	} `json:"item"`
}

func queryChecks(raw []byte, answer string) (bool, bool) {
	tools := false
	answer = strings.ToLower(strings.NewReplacer(",", "", "**", "", "`", "").Replace(answer))
	for _, line := range strings.Split(string(raw), "\n") {
		var event queryEvent
		if json.Unmarshal([]byte(line), &event) != nil {
			continue
		}
		i := event.Item
		if event.Type != "item.completed" || i.Type != "mcp_tool_call" || i.Server != "apple_bridge" || i.Tool != "maps_eta" || i.Status == "failed" || i.Result == nil || i.Result.IsError || len(i.Error) > 0 && string(i.Error) != "null" {
			continue
		}
		var args struct {
			From struct{ Lat, Lon float64 }
			To   struct{ Lat, Lon float64 }
			Mode string
		}
		if json.Unmarshal(i.Arguments.Args, &args) != nil || args.Mode != "driving" || args.From.Lat != 37.7955 || args.From.Lon != -122.3937 || args.To.Lat != 37.8024 || args.To.Lon != -122.4058 {
			continue
		}
		result := i.Result.StructuredContent
		if len(result.ResultJSON) == 0 {
			result = i.Result.CodexStructuredContent
		}
		if len(result.ResultJSON) == 0 {
			for _, content := range i.Result.Content {
				if json.Unmarshal([]byte(content.Text), &result) == nil && len(result.ResultJSON) > 0 {
					break
				}
			}
		}
		var estimate struct{ Duration, Distance float64 }
		if json.Unmarshal(result.ResultJSON, &estimate) != nil || estimate.Duration <= 0 || estimate.Distance <= 0 {
			continue
		}
		tools = true
		duration := regexp.QuoteMeta(strconv.FormatFloat(estimate.Duration, 'f', -1, 64))
		distance := regexp.QuoteMeta(strconv.FormatFloat(estimate.Distance, 'f', -1, 64))
		if strings.Contains(answer, "maps.eta") && strings.Contains(answer, "synthetic") && regexp.MustCompile(`\b`+duration+`\s*(?:milliseconds|ms)\b`).MatchString(answer) && regexp.MustCompile(`\b`+distance+`\s*(?:meters|m)\b`).MatchString(answer) {
			return true, true
		}
	}
	return tools, false
}

func simulatedQuery(ctx context.Context, w *world.World) (outcome, []byte, error) {
	properties := func(schema map[string]any) map[string]any { p, _ := schema["properties"].(map[string]any); return p }
	var schema map[string]any
	for _, descriptor := range contracts.Operations {
		if descriptor.Name == "maps.eta" {
			schema = contracts.OperationSchema(descriptor)
		}
	}
	args, _ := properties(schema)["args"].(map[string]any)
	for _, name := range []string{"from", "to"} {
		point, _ := properties(args)[name].(map[string]any)
		if properties(point)["lat"] == nil || properties(point)["lon"] == nil {
			return outcome{}, nil, fmt.Errorf("Maps ETA must describe the lat and lon fields for each endpoint")
		}
	}

	endpoint := w.Replicas[0]
	token, err := w.ScopedToken(endpoint, "maps:read")
	if err != nil {
		return outcome{}, nil, err
	}
	endpoint.Token = token
	var transcript []byte
	client := scripted.Client{Endpoint: endpoint, Protocol: "mcp", Observe: func(_ http.Header, raw []byte) {
		var envelope struct {
			Result json.RawMessage `json:"result"`
		}
		if json.Unmarshal(raw, &envelope) == nil {
			transcript, _ = json.Marshal(map[string]any{"type": "item.completed", "item": map[string]any{"type": "mcp_tool_call", "server": "apple_bridge", "tool": "maps_eta", "arguments": contracts.Query{Args: json.RawMessage(queryArgs)}, "result": envelope.Result}})
		}
	}}
	var result contracts.QueryResult
	err = client.Call(ctx, "maps.eta", contracts.Query{Args: json.RawMessage(queryArgs), DeadlineMS: 10000}, &result)
	if err != nil {
		return outcome{}, transcript, err
	}
	var estimate struct{ Duration, Distance float64 }
	if err = json.Unmarshal(result.ResultJSON, &estimate); err != nil {
		return outcome{}, transcript, err
	}
	return outcome{Calls: 1, Answer: fmt.Sprintf("maps.eta synthetic: %.0f milliseconds, %.0f meters", estimate.Duration, estimate.Distance)}, transcript, nil
}
