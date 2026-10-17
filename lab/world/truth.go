package world

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

var ErrTruthUnverified = errors.New("AppleTruth access unverified")

func (w *World) Truth(verb string, request map[string]any) (map[string]any, error) {
	id := contracts.NewID("truth")
	input := filepath.Join(w.Dir, id+"-request.json")
	output := filepath.Join(w.Dir, id+"-report.json")
	defer os.Remove(input)
	defer os.Remove(output)
	data, err := json.Marshal(request)
	if err != nil {
		return nil, err
	}
	if err = os.WriteFile(input, data, 0600); err != nil {
		return nil, err
	}
	app := filepath.Join(os.Getenv("HOME"), "Applications", "AppleBridgeDev", "AppleTruth.app")
	if _, err = commandOutput(w.ctx, "open", "-n", "-W", "-g", "-a", app, "--args", verb, "--request-file", input, "--report", output); err != nil {
		return nil, ErrTruthUnverified
	}
	data, err = os.ReadFile(output)
	if err != nil {
		return nil, ErrTruthUnverified
	}
	var response map[string]any
	decoder := json.NewDecoder(bytes.NewReader(data))
	decoder.UseNumber()
	if decoder.Decode(&response) != nil || response["status"] != "verified" {
		return nil, ErrTruthUnverified
	}
	return response, nil
}

func (w *World) RestartHelper() error {
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
