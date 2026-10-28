package shortcut

import (
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"sync"
	"syscall"
	"time"
)

const PackVersion = "1"
const MaxBytes = 16 << 20

type CLI struct{ Executable string }
type Entry struct {
	Name       string `json:"name"`
	Identifier string `json:"identifier"`
}
type Input struct {
	Shortcut   string
	Data       []byte
	Timeout    time.Duration
	OutputType string
}
type Result struct {
	Data       []byte
	ExitStatus int
	TimedOut   bool
}

var inventoryLine = regexp.MustCompile(`^(.*) \(([0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12})\)$`)

func ParseInventory(raw []byte) ([]Entry, error) {
	entries := []Entry{}
	seen := map[string]bool{}
	for _, line := range strings.Split(strings.TrimSuffix(string(raw), "\n"), "\n") {
		if line == "" {
			continue
		}
		match := inventoryLine.FindStringSubmatch(line)
		if match == nil || match[1] == "" || seen[strings.ToLower(match[2])] {
			return nil, fmt.Errorf("invalid shortcuts inventory")
		}
		seen[strings.ToLower(match[2])] = true
		entries = append(entries, Entry{Name: match[1], Identifier: match[2]})
	}
	return entries, nil
}

type boundedOutput struct {
	data     []byte
	exceeded func()
}

func (b *boundedOutput) Write(p []byte) (int, error) {
	if len(b.data)+len(p) > MaxBytes {
		if b.exceeded != nil {
			b.exceeded()
		}
		return 0, fmt.Errorf("shortcut output exceeds limit")
	}
	b.data = append(b.data, p...)
	return len(p), nil
}

func (c CLI) executable() string {
	if c.Executable != "" {
		return c.Executable
	}
	return "/usr/bin/shortcuts"
}

func (c CLI) Inventory(ctx context.Context, folders bool, folder string) ([]Entry, error) {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	args := []string{"list", "--show-identifiers"}
	if folders {
		args = append(args, "--folders")
	}
	if folder != "" {
		args = append(args, "--folder-name", folder)
	}
	var output boundedOutput
	cmd := exec.CommandContext(ctx, c.executable(), args...)
	cmd.Stdout = &output
	output.exceeded = func() {
		if cmd.Process != nil {
			_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
		}
	}
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
	cmd.WaitDelay = time.Second
	if err := cmd.Run(); err != nil {
		return nil, fmt.Errorf("shortcuts inventory unavailable")
	}
	return ParseInventory(output.data)
}

func (c CLI) Run(ctx context.Context, input Input) (Result, error) {
	result := Result{ExitStatus: -1}
	if strings.TrimSpace(input.Shortcut) == "" || strings.HasPrefix(input.Shortcut, "-") || len(input.Data) > MaxBytes || input.Timeout < 0 || input.Timeout > 5*time.Minute {
		return result, fmt.Errorf("invalid shortcut input")
	}
	if input.Timeout == 0 {
		input.Timeout = 30 * time.Second
	}
	ctx, cancel := context.WithTimeout(ctx, input.Timeout)
	defer cancel()
	dir, err := os.MkdirTemp("", "ab-shortcut-")
	if err != nil {
		return result, err
	}
	defer os.RemoveAll(dir)
	output := filepath.Join(dir, "output")
	args := []string{"run", input.Shortcut, "--output-path", output}
	if input.Data != nil {
		path := filepath.Join(dir, "input")
		if err = os.WriteFile(path, input.Data, 0600); err != nil {
			return result, err
		}
		args = append(args, "--input-path", path)
	}
	if input.OutputType != "" {
		args = append(args, "--output-type", input.OutputType)
	}
	cmd := exec.CommandContext(ctx, c.executable(), args...)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	cmd.Cancel = func() error { return syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL) }
	cmd.WaitDelay = time.Second
	var overflow bool
	var overflowMu sync.Mutex
	killOverflow := func() {
		overflowMu.Lock()
		overflow = true
		overflowMu.Unlock()
		if cmd.Process != nil {
			_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
		}
	}
	stdout := boundedOutput{exceeded: killOverflow}
	cmd.Stdout = &stdout
	if err = cmd.Start(); err != nil {
		return result, fmt.Errorf("shortcut process unavailable")
	}
	defer syscall.Kill(-cmd.Process.Pid, syscall.SIGKILL)
	done := make(chan struct{})
	monitored := make(chan struct{})
	go func() {
		defer close(monitored)
		ticker := time.NewTicker(5 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-done:
				return
			case <-ticker.C:
				if info, statErr := os.Lstat(output); statErr == nil && (!info.Mode().IsRegular() || info.Size() > MaxBytes) {
					killOverflow()
					return
				}
			}
		}
	}()
	err = cmd.Wait()
	close(done)
	<-monitored
	overflowMu.Lock()
	exceeded := overflow
	overflowMu.Unlock()
	if exceeded {
		return result, fmt.Errorf("shortcut output exceeds limit")
	}
	result.TimedOut = errors.Is(ctx.Err(), context.DeadlineExceeded)
	if cmd.ProcessState != nil {
		result.ExitStatus = cmd.ProcessState.ExitCode()
	}
	if ctx.Err() != nil {
		return result, ctx.Err()
	}
	if err != nil {
		var exit *exec.ExitError
		if errors.As(err, &exit) {
			return result, nil
		}
		return result, fmt.Errorf("shortcut process unavailable")
	}
	info, err := os.Lstat(output)
	if os.IsNotExist(err) {
		return result, nil
	}
	if err != nil {
		return result, err
	}
	if !info.Mode().IsRegular() || info.Size() > MaxBytes {
		return result, fmt.Errorf("invalid shortcut output")
	}
	file, err := os.Open(output)
	if err != nil {
		return result, err
	}
	defer file.Close()
	result.Data, err = io.ReadAll(io.LimitReader(file, MaxBytes+1))
	if len(result.Data) > MaxBytes {
		result.Data = nil
		return result, fmt.Errorf("shortcut output exceeds limit")
	}
	return result, err
}

func OpenPack(ctx context.Context, directory string) error {
	paths := []string{}
	for _, name := range []string{"ab-echo", "ab-clock", "ab-focus"} {
		path, err := filepath.Abs(filepath.Join(directory, name+".shortcut"))
		if err != nil {
			return err
		}
		f, err := os.Open(path)
		if err != nil {
			return err
		}
		header := make([]byte, 4)
		n, readErr := f.Read(header)
		f.Close()
		if readErr != nil || n != 4 || string(header) != "AEA1" {
			return fmt.Errorf("pack requires signed %s.shortcut", name)
		}
		paths = append(paths, path)
	}
	return exec.CommandContext(ctx, "/usr/bin/open", paths...).Run()
}
