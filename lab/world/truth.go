package world

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"syscall"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

var ErrTruthUnverified = errors.New("AppleTruth access unverified")

func (w *World) Truth(verb string, request map[string]any) (map[string]any, error) {
	return w.truth(w.ctx, verb, request)
}

func (w *World) RecoverTruth(verb string, request map[string]any) (map[string]any, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 95*time.Second)
	defer cancel()
	return w.truth(ctx, verb, request)
}

func (w *World) truth(ctx context.Context, verb string, request map[string]any) (map[string]any, error) {
	id := contracts.NewID("truth")
	input := filepath.Join(w.Dir, id+"-request.json")
	output := filepath.Join(w.Dir, id+"-report.json")
	defer os.Remove(input)

	data, err := json.Marshal(request)
	if err != nil {
		return nil, err
	}
	if err = os.WriteFile(input, data, 0600); err != nil {
		return nil, err
	}
	failure := func(reason string) (map[string]any, error) {
		safe := reason
		if !regexp.MustCompile(`^[A-Za-z_][A-Za-z_0-9.:-]{0,159}$`).MatchString(safe) {
			safe = "native_error_redacted"
		}
		private := filepath.Join(os.Getenv("HOME"), "Library", "Application Support", "AppleBridgeDev", "lab-failures", filepath.Base(w.Dir))
		if err := os.MkdirAll(private, 0700); err == nil {
			os.Chmod(private, 0700)
			for _, path := range []string{input, output} {
				if bytes, err := os.ReadFile(path); err == nil {
					os.WriteFile(filepath.Join(private, filepath.Base(path)), bytes, 0600)
				}
			}
		}
		os.Remove(output)
		w.TruthFailures = append(w.TruthFailures, safe)
		return nil, fmt.Errorf("%w: %s; private report %s", ErrTruthUnverified, safe, filepath.Join("AppleBridgeDev", "lab-failures", filepath.Base(w.Dir), filepath.Base(output)))
	}
	app := w.TruthApp
	_, launchErr := commandOutput(ctx, "open", "-n", "-W", "-g", "-a", app, "--args", verb, "--request-file", input, "--report", output)
	data, err = os.ReadFile(output)
	if err != nil {
		if launchErr != nil {
			return failure("launch_failed")
		}
		return failure("report_unreadable")
	}
	var response map[string]any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if decoder.Decode(&response) != nil {
		return failure("report_invalid")
	}
	if response["status"] != "verified" {
		reason, _ := response["reason"].(string)
		domain, _ := response["error_domain"].(string)
		code, _ := response["error_code"].(json.Number)
		if domain != "" && code != "" && !regexp.MustCompile(`^[A-Za-z_][A-Za-z_0-9.:-]{0,159}$`).MatchString(reason) {
			reason = domain + ":" + string(code)
		}
		if reason == "" {
			reason = "reason_missing"
		}
		return failure(reason)
	}
	os.Remove(output)
	return response, nil
}

func (w *World) restartNativeHelper() error {
	data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	if err != nil {
		return err
	}
	pid := string(data)
	if _, err = commandOutput(w.ctx, "kill", "-KILL", pid); err != nil {
		return err
	}
	return w.Await(10e9, func() bool {
		current, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
		return err == nil && string(current) != pid
	})
}

func (w *World) QuiesceNative() error {
	if !w.Native {
		return nil
	}
	data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
	if err != nil {
		return err
	}
	pid, err := strconv.Atoi(string(data))
	if err != nil || pid <= 1 {
		return fmt.Errorf("invalid world Helper pid")
	}
	for _, process := range w.processes {
		if process != nil && process.home == w.Gateway.Home {
			process.Stop()
		}
	}
	if err = syscall.Kill(pid, syscall.SIGTERM); err != nil && err != syscall.ESRCH {
		return err
	}
	deadline := time.NewTimer(5 * time.Second)
	defer deadline.Stop()
	tick := time.NewTicker(10 * time.Millisecond)
	defer tick.Stop()
	for syscall.Kill(pid, 0) == nil {
		select {
		case <-deadline.C:
			return fmt.Errorf("world Helper did not stop")
		case <-tick.C:
		}
	}
	return nil
}
