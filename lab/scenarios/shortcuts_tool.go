package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"os/exec"
)

func buildShortcutsTool(ctx context.Context) (string, error) {
	output, err := exec.CommandContext(ctx, "rust/check.sh", "build", "-q", "-p", "ab-shortcuts", "--bin", "ab-shortcuts", "--message-format=json").Output()
	if err != nil {
		return "", err
	}
	var executable string
	for _, line := range bytes.Split(output, []byte("\n")) {
		var message struct {
			Reason     string `json:"reason"`
			Executable string `json:"executable"`
			Target     struct {
				Name string `json:"name"`
			} `json:"target"`
		}
		if json.Unmarshal(line, &message) == nil && message.Reason == "compiler-artifact" && message.Target.Name == "ab-shortcuts" && message.Executable != "" {
			executable = message.Executable
		}
	}
	if executable == "" {
		return "", errors.New("ab-shortcuts executable not reported by cargo")
	}
	return executable, nil
}
