package scenarios

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"os"
	"os/exec"
	"strings"
	"time"
)

func runMessagesCommands(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Assertions: []Assertion{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if s.ID == "MSG-SELFTEST" && level == "real" {
		return runNativeMessagesSelfTest(ctx, e)
	}
	if level == "real" && (s.ID == "MSG-READSTATE-REAL" || s.ID == "MSG-DELETE-REAL") && os.Getenv("APPLE_BRIDGE_MESSAGES_UI_DEFERRED") == "1" {
		e.Status = "environment-limited"
		e.Metrics = map[string]any{"writes_submitted": 0, "ui_deferred": true}
		e.Assertions = append(e.Assertions, Assertion{Name: "unlocked_messages_window", Status: "unverified", Detail: "UI proof deferred until the Air is unlocked; no world or command was created"})
		return e
	}
	if level != "sim" {
		return runNativeMessagesCommands(ctx, s, level, e)
	}
	return runMessagesCommandsSim(ctx, s, level, e)
}

func fmtBool(b bool) string {
	if b {
		return "_remove"
	}
	return "_add"
}
