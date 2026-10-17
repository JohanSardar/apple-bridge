package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
	"syscall"
	"time"
)

func runPackaging(ctx context.Context, s Scenario, level string, e Evidence) (result Evidence) {
	result = e
	add := func(name string, ok bool) bool {
		result.Assertions = append(result.Assertions, Assertion{Name: name, Passed: ok})
		return ok
	}
	command := func(name string, args ...string) ([]byte, error) {
		return exec.CommandContext(ctx, name, args...).CombinedOutput()
	}
	if !add("real_macos", level == "real" && runtime.GOOS == "darwin") {
		return
	}
	installed := filepath.Join(os.Getenv("HOME"), "Applications", "Apple Bridge.app")
	if _, err := os.Lstat(installed); !add("test_install_path_unoccupied", os.IsNotExist(err)) {
		return
	}
	domain := fmt.Sprintf("gui/%d/dev.applebridge.gateway", os.Getuid())
	if _, err := command("launchctl", "print", domain); !add("test_agent_unregistered", err != nil) {
		return
	}
	root, err := os.MkdirTemp("/tmp/ablab", "packaging-")
	if !add("isolated_home", err == nil) {
		return
	}
	home := filepath.Join(root, "home")
	helper := filepath.Join(installed, "Contents", "MacOS", "AppleBridgeHelper")
	report := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", filepath.Base(root)+"-permissions.json")
	registered := false
	foregroundPID := 0
	defer func() {
		cleanupContext, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		ctx = cleanupContext
		defer os.Remove(report)
		if foregroundPID > 0 {
			syscall.Kill(foregroundPID, syscall.SIGTERM)
		}
		if registered {
			_, err := command(helper, "uninstall")
			add("smappservice_unregister", err == nil)
			deadline := time.Now().Add(15 * time.Second)
			for time.Now().Before(deadline) {
				if _, err := command("launchctl", "print", domain); err != nil {
					break
				}
				time.Sleep(200 * time.Millisecond)
			}
			_, err = command("launchctl", "print", domain)
			add("agent_removed_from_login_session", err != nil)
		}
		keychain := filepath.Join(home, "Apple Bridge.keychain-db")
		if _, err := os.Stat(keychain); err == nil {
			_, err = command("security", "delete-keychain", keychain)
			add("test_keychain_removed", err == nil)
		}
		add("test_app_removed", os.RemoveAll(installed) == nil)
		add("test_state_removed", os.RemoveAll(root) == nil)
		result.Passed = len(result.Assertions) > 0
		for _, assertion := range result.Assertions {
			result.Passed = result.Passed && assertion.Passed
		}
	}()
	output, err := command("packaging/build-app.sh", installed, "--home", home, "--api-addr", "127.0.0.1:0", "--peer-addr", "127.0.0.1:0")
	if !add("build_signed_installable_app", err == nil) {
		fmt.Fprintln(os.Stderr, string(output))
		return
	}
	_, err = command("codesign", "--verify", "--deep", "--strict", installed)
	if !add("bundle_signature", err == nil) {
		return
	}
	output, err = command("open", "-n", "-W", "-g", "-a", installed, "--args", "status", "--report", report)
	data, readErr := os.ReadFile(report)
	var permissions map[string]string
	parseErr := json.Unmarshal(data, &permissions)
	if !add("onboarding_status_uses_existing_permissions", err == nil && readErr == nil && parseErr == nil && len(permissions) >= 7) {
		fmt.Fprintln(os.Stderr, string(output))
		return
	}
	output, err = command(helper, "install")
	registered = true
	if !add("smappservice_registered", err == nil) {
		fmt.Fprintln(os.Stderr, string(output))
		return
	}
	agentPID := func() int {
		output, err := command("launchctl", "print", domain)
		if err != nil || !strings.Contains(string(output), "state = running") {
			return 0
		}
		for _, line := range strings.Split(string(output), "\n") {
			fields := strings.Fields(line)
			if len(fields) == 3 && fields[0] == "pid" && fields[1] == "=" {
				pid, _ := strconv.Atoi(fields[2])
				return pid
			}
		}
		return 0
	}
	health := func() bool {
		data, err := os.ReadFile(filepath.Join(home, "runtime.json"))
		var config struct {
			APIAddress string `json:"api_address"`
		}
		if err != nil || json.Unmarshal(data, &config) != nil || config.APIAddress == "" {
			return false
		}
		client := http.Client{Timeout: time.Second}
		response, err := client.Get("http://" + config.APIAddress + "/v1/health")
		if err != nil {
			return false
		}
		defer response.Body.Close()
		body, _ := io.ReadAll(response.Body)
		return response.StatusCode == 200 && strings.Contains(string(body), "alive")
	}
	await := func(check func() bool) bool {
		deadline := time.Now().Add(40 * time.Second)
		for time.Now().Before(deadline) && ctx.Err() == nil {
			if check() {
				return true
			}
			time.Sleep(200 * time.Millisecond)
		}
		return false
	}
	if !add("launchctl_running_and_gateway_health", await(func() bool { return agentPID() > 0 && health() })) {
		return
	}
	passwordInfo, err := os.Stat(filepath.Join(home, "keychain-password"))
	if !add("dedicated_keychain_password_private", err == nil && passwordInfo.Mode().Perm() == 0600) {
		return
	}
	if !add("dedicated_keychain_created", func() bool { _, err := os.Stat(filepath.Join(home, "Apple Bridge.keychain-db")); return err == nil }()) {
		return
	}
	if s.ID == "PKG-BACKGROUND" {
		_, err = command("open", "-n", "-g", "-a", installed)
		if !add("foreground_app_launched", err == nil) {
			return
		}
		time.Sleep(time.Second)
		output, err := command("pgrep", "-f", "^"+helper+"$")
		fields := strings.Fields(string(output))
		if !add("foreground_app_identified", err == nil && len(fields) == 1) {
			return
		}
		foreground, _ := strconv.Atoi(fields[0])
		foregroundPID = foreground
		if !add("quit_foreground_app", syscall.Kill(foreground, syscall.SIGTERM) == nil) {
			return
		}
		if !add("gateway_survives_app_quit", await(func() bool { return syscall.Kill(foreground, 0) != nil && health() && agentPID() > 0 })) {
			return
		}
		previous := agentPID()
		_, err = command("security", "lock-keychain", filepath.Join(home, "Apple Bridge.keychain-db"))
		if !add("lock_dedicated_keychain_before_restart", err == nil) {
			return
		}
		if !add("kill_gateway", syscall.Kill(previous, syscall.SIGKILL) == nil) {
			return
		}
		if !add("launchd_restarts_gateway_and_unlocks_credentials", await(func() bool { pid := agentPID(); return pid > 0 && pid != previous && health() })) {
			return
		}
	}
	return
}
