package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/agent"
)

func runQueryAgent(ctx context.Context, s Scenario, level string) Evidence {
	err := agent.Query(ctx, level, false)
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC()}
	var proof struct {
		GitSHA string `json:"git_sha"`
		Passed bool   `json:"passed"`
		Tools  bool   `json:"tools_check"`
		Answer bool   `json:"answer_check"`
	}
	raw, readErr := os.ReadFile(filepath.Join("lab/evidence", level, s.ID+".json"))
	decoded := readErr == nil && json.Unmarshal(raw, &proof) == nil
	e.GitSHA = proof.GitSHA
	e.Assertions = []Assertion{{Name: "agent_execution", Passed: err == nil && decoded && proof.Passed}, {Name: "maps_tool_result", Passed: decoded && proof.Tools}, {Name: "answer_uses_tool_result", Passed: decoded && proof.Answer}}
	e.Passed = err == nil && decoded && proof.Passed && proof.Tools && proof.Answer
	return e
}
