package remote

import (
	"context"
	"fmt"
	"io"
	"net/url"
	"os/exec"
	"runtime"
	"time"
)

func openLink(ctx context.Context, link string) error {
	u, err := url.Parse(link)
	if err != nil || u.Scheme != "https" || u.Host == "" {
		return fmt.Errorf("invalid approval link")
	}
	launcher := "open"
	if runtime.GOOS != "darwin" {
		launcher = "xdg-open"
	}
	bounded, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	return exec.CommandContext(bounded, launcher, link).Run()
}

func Snippets(home string, out io.Writer) error {
	var status Status
	if err := read(home, "status", &status); err != nil {
		return err
	}
	if status.State != "on" || status.URL == "" {
		return fmt.Errorf("remote is not ready")
	}
	_, err := fmt.Fprintf(out, "Claude Code: claude mcp add --transport http apple-bridge %s/mcp\nclaude.ai connector: %s/mcp\nChatGPT connector: %s/mcp\nCodex config.toml:\n[mcp_servers.apple-bridge]\nurl = %q\nGrok connector URL: %s/mcp\n", status.URL, status.URL, status.URL, status.URL+"/mcp", status.URL)
	if err != nil {
		return err
	}
	_, err = fmt.Fprintf(out, "Grok (xai_sdk.tools): mcp(server_url=%q, server_label=%q, authorization=%q)\n", status.URL+"/mcp", "apple-bridge", "Bearer <agent-token>")
	return err
}
