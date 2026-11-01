package scenarios

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"strings"
)

func lockAirFocus(ctx context.Context) (func(), error) {
	remote := `lockf -k "$HOME/.ab-locks/app-focus" `
	if os.Getenv("AB_REAL_FOCUS_AIR_PACK") != "" {
		remote += `lockf -k "$HOME/.ab-locks/app-shortcuts" `
	}
	remote += `sh -c 'printf "locked\n"; cat >/dev/null'`
	command := exec.CommandContext(ctx, "ssh", "-o", "BatchMode=yes", "air", remote)
	input, err := command.StdinPipe()
	if err != nil {
		return nil, err
	}
	output, err := command.StdoutPipe()
	if err != nil {
		input.Close()
		return nil, err
	}
	command.Stderr = os.Stderr
	if err = command.Start(); err != nil {
		input.Close()
		return nil, err
	}
	release := func() { input.Close(); command.Wait() }
	line, err := bufio.NewReader(output).ReadString('\n')
	if err != nil || line != "locked\n" {
		release()
		return nil, fmt.Errorf("The Air Focus lock could not be acquired.")
	}
	return release, nil
}

func runAirFocusReal(ctx context.Context) Evidence {
	e := Evidence{Scenario: "FOC-PHONE-SYNC", Level: "real", Passed: true}
	check := func(name string, pass bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: pass})
		e.Passed = e.Passed && pass
	}
	script, err := os.ReadFile("lab/iphone-real/focus-air.sh")
	check("air_focus_step", err == nil)
	if err != nil {
		return e
	}
	remote := "sh -s"
	helper, pack := os.Getenv("AB_REAL_FOCUS_AIR_HELPER"), os.Getenv("AB_REAL_FOCUS_AIR_PACK")
	if helper != "" || pack != "" {
		remote += " -- '" + strings.ReplaceAll(helper, "'", "'\"'\"'") + "' '" + strings.ReplaceAll(pack, "'", "'\"'\"'") + "'"
	}
	command := exec.CommandContext(ctx, "ssh", "-o", "BatchMode=yes", "air", remote)
	command.Stdin = strings.NewReader(string(script))
	command.Stderr = os.Stderr
	raw, err := command.Output()
	check("air_signed_set_restore_probe", err == nil)
	if err != nil {
		return e
	}
	return airFocusEvidence(e, raw)
}

func airFocusEvidence(e Evidence, raw []byte) Evidence {
	var report map[string]any
	err := json.Unmarshal(raw, &report)
	for _, item := range []struct{ name, key string }{
		{"air_helper_report", ""},
		{"air_focus_status_authorized", "grant_passed"},
		{"air_focus_initially_inactive", "inactive_passed"},
		{"air_focus_set_readback", "set_passed"},
		{"air_focus_restore_readback", "restore_passed"},
	} {
		pass := err == nil && (item.key == "" || report[item.key] == true)
		e.Assertions = append(e.Assertions, Assertion{Name: item.name, Passed: pass})
		e.Passed = e.Passed && pass
	}
	return e
}
