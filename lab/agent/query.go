package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func Query(ctx context.Context, asJSON bool) error {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	if err := world.Build(ctx, "replica"); err != nil {
		return err
	}
	w, err := world.New(ctx, "replica", 1, 0, world.BlobOptions{Scope: "maps:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	driver, err := os.ReadFile("lab/agent/codex.py")
	if err != nil {
		return err
	}
	private := filepath.Join(w.Dir, "agent-private")
	runner := AgentRunner{Driver: driver, Private: private}
	prompt := `Use only apple_bridge MCP tools. What's the current weather at San Francisco Ferry Building (latitude 37.7955, longitude -122.3937) and the driving ETA from there to Coit Tower (latitude 37.8024, longitude -122.4058)? Call weather_current with latitude/longitude arguments and maps_eta with from/to lat/lon points and mode driving. These are synthetic providers, not live conditions. Cite both operation names (weather.current and maps.eta) in your answer. Report temperature in Celsius and ETA duration in milliseconds, preserving the returned units. Include weather condition and attribution.`
	out, runErr := runner.execute(ctx, w, Task{ID: "QUERY-AGENT", Scope: "maps:read,system:read"}, 1, prompt, []string{"weather_current", "maps_eta"})
	transcript, readErr := os.ReadFile(filepath.Join(private, "QUERY-AGENT-1", "transcript.jsonl"))
	tools := queryTools(transcript)
	answer := strings.ToLower(out.Answer)
	answerOK := true
	for _, required := range []string{"weather.current", "maps.eta", "cloudy", "synthetic"} {
		answerOK = answerOK && strings.Contains(answer, required)
	}
	answerOK = answerOK && regexp.MustCompile(`\b12\b`).MatchString(answer) && regexp.MustCompile(`\b600000\b`).MatchString(answer)
	passed := runErr == nil && readErr == nil && out.Exit == 0 && out.Calls >= 2 && out.Forbidden == 0 && tools && answerOK
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	evidence := map[string]any{"at": time.Now().UTC(), "git_sha": strings.TrimSpace(string(sha)), "scenario": "QUERY-AGENT", "level": "agent", "passed": passed, "mcp_calls": out.Calls, "forbidden_calls": out.Forbidden, "tools_check": tools, "answer_check": answerOK}
	if !passed {
		evidence["failure"] = "agent_execution_or_answer"
	}
	raw, err := json.MarshalIndent(evidence, "", "  ")
	if err != nil {
		return err
	}
	if err = os.MkdirAll("lab/evidence/agent", 0700); err != nil {
		return err
	}
	if err = os.WriteFile("lab/evidence/agent/QUERY-AGENT.json", append(raw, '\n'), 0600); err != nil {
		return err
	}
	if asJSON {
		fmt.Println(string(raw))
	}
	if !passed {
		return fmt.Errorf("QUERY-AGENT failed")
	}
	return nil
}

func queryTools(raw []byte) bool {
	seen := map[string]bool{}
	for _, line := range strings.Split(string(raw), "\n") {
		var event struct {
			Type string `json:"type"`
			Item struct {
				Type      string `json:"type"`
				Server    string `json:"server"`
				Tool      string `json:"tool"`
				Arguments struct {
					Args json.RawMessage `json:"args"`
				} `json:"arguments"`
				Error  json.RawMessage `json:"error"`
				Result *struct {
					IsError bool `json:"isError"`
				} `json:"result"`
			} `json:"item"`
		}
		if json.Unmarshal([]byte(line), &event) != nil {
			continue
		}
		i := event.Item
		if event.Type != "item.completed" || i.Type != "mcp_tool_call" || i.Server != "apple_bridge" || i.Result == nil || i.Result.IsError || len(i.Error) > 0 && string(i.Error) != "null" {
			continue
		}
		var args map[string]any
		if json.Unmarshal(i.Arguments.Args, &args) != nil {
			continue
		}
		point := func(v any, lat, lon float64) bool {
			p, ok := v.(map[string]any)
			return ok && p["lat"] == lat && p["lon"] == lon
		}
		if i.Tool == "weather_current" && args["latitude"] == 37.7955 && args["longitude"] == -122.3937 {
			seen[i.Tool] = true
		}
		if i.Tool == "maps_eta" && args["mode"] == "driving" && point(args["from"], 37.7955, -122.3937) && point(args["to"], 37.8024, -122.4058) {
			seen[i.Tool] = true
		}
	}
	return seen["weather_current"] && seen["maps_eta"]
}
