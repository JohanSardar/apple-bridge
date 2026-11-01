package scenarios

import (
	"context"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"time"
)

func simulatedIntentAppReplacement() bool {
	directory, err := os.MkdirTemp("", "ab-intent-replacement-")
	if err != nil {
		return false
	}
	defer os.RemoveAll(directory)
	app := filepath.Join(directory, "current.app")
	installed := filepath.Join(directory, "installed.app")
	previous := filepath.Join(directory, "previous.app")
	if os.Mkdir(app, 0700) != nil || os.Mkdir(installed, 0700) != nil || os.WriteFile(filepath.Join(app, "current"), []byte("synthetic"), 0600) != nil {
		return false
	}
	if replaceBridgeIntentApp(app, installed, previous) != nil {
		return false
	}
	data, err := os.ReadFile(filepath.Join(installed, "current"))
	_, previousErr := os.Stat(previous)
	return err == nil && string(data) == "synthetic" && os.IsNotExist(previousErr)
}

func simulatedIntentHostCleanup(ctx context.Context) bool {
	directory, err := os.MkdirTemp("", "ab-intent-host-check-")
	if err != nil {
		return false
	}
	defer os.RemoveAll(directory)
	executable := filepath.Join(directory, "[ab-test] intent helper")
	var owned []*exec.Cmd
	var completions []chan struct{}
	for _, name := range []string{executable, executable, filepath.Join(directory, "unrelated helper")} {
		command := exec.Command("/bin/sleep", "30")
		command.Args[0] = name
		if command.Start() != nil {
			return false
		}
		done := make(chan struct{})
		go func() { _ = command.Wait(); close(done) }()
		defer func() { _ = command.Process.Kill(); <-done }()
		owned = append(owned, command)
		completions = append(completions, done)
	}
	if stopIntentExecutables(ctx, executable) != nil {
		return false
	}
	for _, done := range completions[:2] {
		select {
		case <-done:
		case <-time.After(time.Second):
			return false
		}
	}
	return owned[2].Process.Signal(syscall.Signal(0)) == nil
}
