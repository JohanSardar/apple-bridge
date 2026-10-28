package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func Mail(ctx context.Context, asJSON bool) error {
	ctx, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	if err := world.Build(ctx, "replica"); err != nil {
		return err
	}
	w, err := world.New(ctx, "replica", 1, 0, world.BlobOptions{Scope: "mail:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	tag := contracts.NewID("mailtag")
	data, _ := json.Marshal(map[string]any{"account_id": "synthetic@example.com", "message_id": "<fixture@example.com>", "read": false, "to": nil, "cc": nil, "sent_at": nil, "flagged": nil, "flag_color": nil, "size": nil, "snippet": nil, "subject": "[ab-test " + tag + "] synthetic email", "from": "sender@example.com", "has_attachments": nil, "body_state": nil, "received_at": "2026-10-28T00:00:00Z", "mailbox_ids": []string{"mailbox/1"}})
	observations := []contracts.Observation{{Kind: "upsert", Collection: "mail.message", NativeKey: tag, Data: data, ObservedAt: time.Now().UTC()}}
	if err = w.ControlAction("/dataset", map[string]any{"records": observations}); err != nil {
		return err
	}
	if err = w.Await(20*time.Second, func() bool {
		records, err := w.Records(w.Replicas[0])
		for _, r := range records {
			if r.Collection == "mail.message" && strings.Contains(string(r.Data), tag) {
				return err == nil
			}
		}
		return false
	}); err != nil {
		return err
	}
	driver, err := os.ReadFile("lab/agent/codex.py")
	if err != nil {
		return err
	}
	private := filepath.Join(w.Dir, "agent-private")
	runner := AgentRunner{Driver: driver, Private: private}
	outcome, err := runner.execute(ctx, w, Task{ID: "MAIL-AGENT", Scope: "mail:read"}, 1, "Use only apple_bridge MCP tools to find the email whose subject contains "+tag+". Return its complete subject and nothing else.", nil)
	passed := err == nil && outcome.Exit == 0 && outcome.Calls > 0 && outcome.Forbidden == 0 && strings.TrimSpace(outcome.Answer) == "[ab-test "+tag+"] synthetic email"
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	evidence := map[string]any{"at": time.Now().UTC(), "git_sha": strings.TrimSpace(string(sha)), "scenario": "MAIL-AGENT", "level": "agent", "passed": passed, "exit_code": outcome.Exit, "mcp_calls": outcome.Calls, "forbidden_calls": outcome.Forbidden, "mcp_only": outcome.Calls > 0 && outcome.Forbidden == 0}
	if !passed {
		evidence["failure"] = "agent_execution_or_answer"
		if log, readErr := os.ReadFile(filepath.Join(private, "MAIL-AGENT-1", "stderr.log")); readErr == nil && strings.Contains(string(log), "token_revoked") {
			evidence["failure"] = "agent_oauth_token_revoked"
		}
	}
	raw, _ := json.MarshalIndent(evidence, "", "  ")
	if err = os.MkdirAll("lab/evidence/agent", 0700); err != nil {
		return err
	}
	if err = os.WriteFile("lab/evidence/agent/MAIL-AGENT.json", append(raw, '\n'), 0600); err != nil {
		return err
	}
	if asJSON {
		fmt.Println(string(raw))
	}
	if !passed {
		return fmt.Errorf("MAIL-AGENT failed")
	}
	return nil
}
