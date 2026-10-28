package scenarios

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

func runShortcutsRunner(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true, Metrics: map[string]any{}}
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
		e.Passed = e.Passed && passed
	}
	dir, err := os.MkdirTemp("", "ab-shortcuts-fixture-")
	add("fixture_directory", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	executable := filepath.Join(dir, "shortcuts")
	script := `#!/bin/sh
if [ "$1" = list ]; then
 if [ -f "$(dirname "$0")/changed" ]; then
  echo 'Changed (00000000-0000-0000-0000-000000000002)'
 else
  echo 'Synthetic (00000000-0000-0000-0000-000000000001)'
 fi
 exit 0
fi
name="$2"
shift 2
input=""
output=""
while [ "$#" -gt 0 ]; do
 case "$1" in
 --input-path) input="$2"; shift 2;;
 --output-path) output="$2"; shift 2;;
 *) shift;;
 esac
done
case "$name" in
 fail) exit 7;;
 timeout) echo $$ > "$(dirname "$0")/pid"; exec sleep 30;;
 binary) printf '\000\001\377' > "$output";;
 *) cat "$input" > "$output";;
esac
`
	err = os.WriteFile(executable, []byte(script), 0700)
	add("fake_cli", err == nil)
	if err != nil {
		return e
	}
	cli := shortcut.CLI{Executable: executable}
	if s.ID == "SC-CAPTURE" {
		first, err := cli.Inventory(ctx, false, "")
		add("initial_inventory", err == nil && len(first) == 1 && first[0].Name == "Synthetic" && first[0].Identifier == "00000000-0000-0000-0000-000000000001")
		err = os.WriteFile(filepath.Join(dir, "changed"), nil, 0600)
		add("change_fixture", err == nil)
		second, err := cli.Inventory(ctx, false, "")
		add("changed_inventory", err == nil && len(second) == 1 && second[0].Name == "Changed" && second[0].Identifier == "00000000-0000-0000-0000-000000000002")
		_, err = shortcut.ParseInventory([]byte("ambiguous name\n"))
		add("malformed_inventory_rejected", err != nil)
		_, err = shortcut.ParseInventory([]byte("a (00000000-0000-0000-0000-000000000001)\nb (00000000-0000-0000-0000-000000000001)\n"))
		add("duplicate_identifiers_rejected", err != nil)
		return e
	}
	for _, input := range [][]byte{[]byte("[ab-test synthetic]"), []byte("https://example.com/synthetic"), {0, 1, 255}} {
		result, err := cli.Run(ctx, shortcut.Input{Shortcut: "echo", Data: input})
		add("input_output_bytes", err == nil && result.ExitStatus == 0 && string(result.Data) == string(input))
	}
	result, err := cli.Run(ctx, shortcut.Input{Shortcut: "binary"})
	add("binary_output", err == nil && string(result.Data) == string([]byte{0, 1, 255}))
	result, err = cli.Run(ctx, shortcut.Input{Shortcut: "fail"})
	add("nonzero_exit_preserved", err == nil && result.ExitStatus == 7)
	start := time.Now()
	result, err = cli.Run(ctx, shortcut.Input{Shortcut: "timeout", Timeout: 100 * time.Millisecond})
	add("timeout_bounded", errors.Is(err, context.DeadlineExceeded) && result.TimedOut && time.Since(start) < 3*time.Second)
	pidRaw, pidErr := os.ReadFile(filepath.Join(dir, "pid"))
	pid, parseErr := strconv.Atoi(strings.TrimSpace(string(pidRaw)))
	gone := false
	if pidErr == nil && parseErr == nil && pid > 0 {
		gone = errors.Is(syscall.Kill(pid, 0), syscall.ESRCH)
	}
	add("timeout_process_reaped", result.ExitStatus == -1 && gone)
	_, err = cli.Run(ctx, shortcut.Input{Shortcut: "--help"})
	add("option_injection_rejected", err != nil)
	_, err = cli.Run(ctx, shortcut.Input{Shortcut: "echo", Data: make([]byte, shortcut.MaxBytes+1)})
	add("oversized_input_rejected", err != nil)
	return e
}
