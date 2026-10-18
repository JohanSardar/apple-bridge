package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCredentialStore(ctx context.Context, e Evidence) Evidence {
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	w, err := world.New(ctx, "sim", 0, 0)
	if err != nil {
		add("explicit_private_world_credentials", false)
		return e
	}
	defer w.Close()
	files, err := os.ReadDir(filepath.Join(w.Gateway.Home, "secrets"))
	private := err == nil && len(files) > 0
	for _, entry := range files {
		info, err := entry.Info()
		private = private && err == nil && info.Mode().Perm() == 0600
	}
	add("explicit_private_world_credentials", private)
	if runtime.GOOS == "darwin" {
		root := filepath.Join(w.Dir, "locked-store")
		mock := filepath.Join(root, "bin")
		err := os.MkdirAll(mock, 0700)
		if err == nil {
			err = os.WriteFile(filepath.Join(mock, "python3"), []byte("#!/bin/sh\nprintf '%s\\n' '{\"status\":-25308}'\n"), 0700)
		}
		if err != nil {
			add("locked_store_fixture", false)
			return e
		}
		home := filepath.Join(root, "home")
		binary, _ := filepath.Abs("bin/bridge")
		env := []string{}
		for _, entry := range os.Environ() {
			if !strings.HasPrefix(entry, "APPLE_BRIDGE_SECRET_STORE=") && !strings.HasPrefix(entry, "PATH=") {
				env = append(env, entry)
			}
		}
		env = append(env, "PATH="+mock+string(os.PathListSeparator)+os.Getenv("PATH"))
		bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
		command := exec.CommandContext(bounded, binary, "start", "--home", home, "--role", "gateway", "--api-addr", "127.0.0.1:0", "--peer-addr", "127.0.0.1:0")
		command.Env = env
		output, startErr := command.CombinedOutput()
		cancel()
		add("locked_keychain_startup_typed_error_and_remedy", startErr != nil && strings.Contains(string(output), "credential_store_locked") && strings.Contains(string(output), "keychain") && strings.Contains(string(output), "Unlock"))
		command = exec.CommandContext(ctx, binary, "doctor", "--home", home)
		command.Env = env
		output, doctorErr := command.Output()
		var report struct {
			OK          bool `json:"ok"`
			Credentials struct {
				Store     string `json:"store"`
				LastError struct {
					Code   string `json:"code"`
					Remedy string `json:"remedy"`
				} `json:"last_error"`
			} `json:"credentials"`
		}
		parseErr := json.Unmarshal(output, &report)
		add("doctor_reports_locked_keychain_and_remedy", doctorErr == nil && parseErr == nil && !report.OK && report.Credentials.Store == "keychain" && report.Credentials.LastError.Code == "credential_store_locked" && strings.Contains(report.Credentials.LastError.Remedy, "Unlock"))
	}
	e.Passed = true
	for _, assertion := range e.Assertions {
		e.Passed = e.Passed && assertion.Passed
	}
	return e
}
