package world

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

func (w *World) Effects() (map[string]int, error) {
	resp, err := w.Control.Get("http://sim/effects")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var out map[string]int
	err = json.NewDecoder(resp.Body).Decode(&out)
	return out, err
}
func (w *World) StopHelper() {
	for _, p := range w.processes {
		if p != nil && len(p.command.Args) > 1 && p.command.Args[1] == "simapple" {
			p.Stop()
		}
	}
}
func (w *World) RestartHelper() error {
	w.StopHelper()
	cfg, err := w.config(w.Gateway.Home, false)
	if err != nil {
		return err
	}
	_, err = w.start("simapple-restart", false, w.Dir, w.lab, "simapple", "--socket", cfg.Socket, "--secret-file", filepath.Join(w.Gateway.Home, "apple-secret"), "--control", filepath.Join(w.Dir, "sim.sock"))
	if err != nil {
		return err
	}
	return w.Await(10*time.Second, func() bool { _, err := w.Inspect(); return err == nil })
}
func (w *World) ClockOffset(ms int64) error {
	return os.WriteFile(filepath.Join(w.Gateway.Home, "clock-offset"), []byte(strconv.FormatInt(ms, 10)), 0600)
}
func (w *World) StopGateway() { w.processes[0].Stop() }
func (w *World) RestartGateway() error {
	var cfg Config
	b, err := os.ReadFile(filepath.Join(w.Gateway.Home, "runtime.json"))
	if err != nil {
		return err
	}
	if err = json.Unmarshal(b, &cfg); err != nil {
		return err
	}
	w.StopGateway()
	if err = os.Remove(filepath.Join(w.Gateway.Home, "runtime.json")); err != nil {
		return err
	}
	_, err = w.start("gateway-restart", false, w.Gateway.Home, w.bridge, "start", "--role", "gateway", "--home", w.Gateway.Home, "--peer-addr", cfg.PeerAddress, "--api-addr", cfg.APIAddress)
	if err != nil {
		return err
	}
	_, err = w.config(w.Gateway.Home, false)
	return err
}

func (w *World) DropCommandResponse() error {
	return os.WriteFile(filepath.Join(w.Gateway.Home, "drop-command-response"), []byte("once"), 0600)
}

func (w *World) DelayCommandApply(index int, duration time.Duration) error {
	endpoint := w.Replicas[index]
	path := filepath.Join(endpoint.Home, "command-apply-delay")
	if endpoint.Remote {
		script := "import pathlib; pathlib.Path(" + strconv.Quote(path) + ").write_text(" + strconv.Quote(strconv.FormatInt(duration.Milliseconds(), 10)) + ")"
		_, err := w.execute(true, "python3", "-c", script)
		return err
	}
	return os.WriteFile(path, []byte(strconv.FormatInt(duration.Milliseconds(), 10)), 0600)
}
