package world

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"syscall"
	"time"
)

func (p *Process) Signal(signal syscall.Signal) error {
	if p.remote {
		name := map[syscall.Signal]string{syscall.SIGINT: "INT", syscall.SIGTERM: "TERM", syscall.SIGKILL: "KILL", syscall.SIGSTOP: "STOP", syscall.SIGCONT: "CONT"}[signal]
		if name == "" {
			return os.ErrInvalid
		}
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_, err := commandOutput(ctx, "ssh", "ubuntu", "kill -"+name+" $(cat "+quote(filepath.Join(p.home, "pid"))+")")
		return err
	}
	return p.command.Process.Signal(signal)
}

func (w *World) SignalGateway(signal syscall.Signal) error {
	for _, p := range w.processes {
		if p != nil && p.home == w.Gateway.Home && !p.stopped {
			return p.command.Process.Signal(signal)
		}
	}
	return os.ErrProcessDone
}
func (w *World) StopGateway() {
	for _, p := range w.processes {
		if p != nil && p.home == w.Gateway.Home {
			p.Stop()
		}
	}
}
func (w *World) RestartGateway(retentionRows ...int) error {
	rows := 0
	if len(retentionRows) > 0 {
		rows = retentionRows[0]
	}
	return w.RestartGatewayRetention(rows, time.Millisecond)
}
func (w *World) RestartGatewayRetention(rows int, age time.Duration) error {
	var cfg Config
	data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err != nil {
		return err
	}
	if err = json.Unmarshal(data, &cfg); err != nil {
		return err
	}
	w.StopGateway()
	os.Remove(filepath.Join(w.Gateway.Home, "runtime.json"))
	args := []string{"start", "--role", "gateway", "--home", w.Gateway.Home, "--peer-addr", cfg.PeerAddress, "--api-addr", cfg.APIAddress, "--socket", cfg.Socket, "--blob-grace-ms", strconv.FormatInt(w.blobOptions.GraceMS, 10)}
	if cfg.HelperApp != "" {
		args = append(args, "--helper-app", cfg.HelperApp)
	}
	if cfg.HelperLabCommands {
		args = append(args, "--helper-lab-commands")
	}
	if rows > 0 {
		args = append(args, "--retention-rows", strconv.Itoa(rows), "--retention-age", age.String())
	}
	process, err := w.start("gateway", false, w.Gateway.Home, w.bridge, args...)
	if err != nil {
		return err
	}
	w.Gateway.Binary = process.command.Path
	_, err = w.config(w.Gateway.Home, false)
	return err
}
func (w *World) Backup(path string) error {
	binary := w.endpointBinary(w.Gateway)
	if filepath.Base(binary) == "bridge-feed1" {
		binary = w.goBridge
	}
	_, err := w.execute(false, binary, "backup", path, "--home", w.Gateway.Home)
	return err
}
func (w *World) Restore(path string) error {
	_, err := w.execute(false, w.bridge, "restore", path, "--home", w.Gateway.Home)
	return err
}
func (w *World) Doctor(endpoint Endpoint) (map[string]any, error) {
	binary := w.endpointBinary(endpoint)
	if filepath.Base(binary) == "bridge-feed1" {
		binary = w.goBridge
	}
	output, err := w.execute(endpoint.Remote, binary, "doctor", "--home", endpoint.Home)
	var result map[string]any
	if err == nil {
		err = json.Unmarshal([]byte(output), &result)
	}
	return result, err
}

func (w *World) CrashReplica(index int) {
	w.replicaProcesses[index].Signal(syscall.SIGKILL)
	w.StopReplica(index)
}
func (w *World) PauseReplica(index int) error {
	return w.replicaProcesses[index].Signal(syscall.SIGSTOP)
}
func (w *World) ResumeReplica(index int) error {
	return w.replicaProcesses[index].Signal(syscall.SIGCONT)
}
func (w *World) DelayNetwork(delay time.Duration) { w.proxy.Delay(delay) }
func (w *World) CutNetwork(bytes int)             { w.proxy.CutAfter(bytes) }

func (w *World) StopHelper() {
	for _, p := range w.processes {
		if p != nil && p.command.Path == w.simAppleBinary() && !p.stopped {
			p.command.Process.Signal(syscall.SIGKILL)
			p.Stop()
		}
	}
}
func (w *World) RestartHelper() error {
	if w.Native {
		return w.restartNativeHelper()
	}
	previousPID := 0
	for _, p := range w.processes {
		if p != nil && p.command.Path == w.simAppleBinary() && !p.stopped {
			previousPID = p.command.Process.Pid
		}
	}
	w.Control.CloseIdleConnections()
	w.StopHelper()
	var cfg Config
	data, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err != nil {
		return err
	}
	if err = json.Unmarshal(data, &cfg); err != nil {
		return err
	}
	restarted, err := w.start("simapple", false, w.Dir, w.simAppleBinary(), "simapple", "--socket", cfg.Socket, "--secret-file", filepath.Join(w.Gateway.Home, "apple-secret"), "--control", filepath.Join(w.Dir, "sim.sock"))
	if err != nil {
		return err
	}
	if restarted.command.Process.Pid == previousPID {
		return fmt.Errorf("helper process did not restart")
	}
	return w.Await(10*time.Second, func() bool { _, err := w.Inspect(); return err == nil })
}
func (w *World) HelperSourceScans() (map[string]int, error) {
	resp, err := w.Control.Get("http://sim/stats")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var stats struct {
		SourceScans map[string]int `json:"source_scans"`
	}
	err = json.NewDecoder(resp.Body).Decode(&stats)
	return stats.SourceScans, err
}

func (w *World) HelperBlobsIntact() bool {
	response, err := w.Control.Get("http://sim/stats")
	if err != nil {
		return false
	}
	defer response.Body.Close()
	var stats struct {
		Digests map[string]string `json:"blob_digests"`
	}
	if json.NewDecoder(response.Body).Decode(&stats) != nil || len(stats.Digests) == 0 {
		return false
	}
	for digest, actual := range stats.Digests {
		if digest != actual {
			return false
		}
	}
	return true
}

func (w *World) simAppleBinary() string {
	if binary := os.Getenv("AB_LAB_SIMAPPLE"); binary != "" {
		return binary
	}
	return w.lab
}
