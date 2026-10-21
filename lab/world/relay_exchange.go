package world

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

func (w *World) RelayExchange(action string, input any, next func() (any, error), output any) error {
	command := exec.CommandContext(w.ctx, "ssh", "ubuntu", "env APPLE_BRIDGE_SECRET_STORE=file "+quote(filepath.Join(w.Dir, "relayprobe"))+" "+quote(action))
	stdin, err := command.StdinPipe()
	if err != nil {
		return err
	}
	stdout, err := command.StdoutPipe()
	if err != nil {
		stdin.Close()
		return err
	}
	if err = command.Start(); err != nil {
		stdin.Close()
		return err
	}
	defer func() { stdin.Close(); command.Process.Kill(); command.Wait() }()
	encoder := json.NewEncoder(stdin)
	decoder := json.NewDecoder(stdout)
	decode := func(output any) error {
		var raw json.RawMessage
		if err := decoder.Decode(&raw); err != nil {
			return err
		}
		var failure struct {
			Error string `json:"error"`
		}
		if err := json.Unmarshal(raw, &failure); err != nil {
			return err
		}
		if failure.Error != "" {
			os.WriteFile(filepath.Join(w.Dir, "relay-probe.log"), raw, 0600)
			return fmt.Errorf("relay probe failed; private relay-probe.log")
		}
		return json.Unmarshal(raw, output)
	}
	if err = encoder.Encode(input); err != nil {
		return err
	}
	var ready struct {
		Ready bool `json:"ready"`
	}
	if err = decode(&ready); err != nil || !ready.Ready {
		return fmt.Errorf("relay probe not ready")
	}
	value, err := next()
	if err != nil {
		return err
	}
	if err = encoder.Encode(value); err != nil {
		return err
	}
	return decode(output)
}
