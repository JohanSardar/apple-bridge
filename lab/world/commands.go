package world

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
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
func (w *World) ClockOffset(ms int64) error {
	return os.WriteFile(filepath.Join(w.Gateway.Home, "clock-offset"), []byte(strconv.FormatInt(ms, 10)), 0600)
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

func (w *World) CommandBackup(path string) error {
	_, err := w.execute(false, w.bridge, "backup", path, "--home", w.Gateway.Home)
	return err
}
func (w *World) CommandRestore(path string, accept bool) error {
	args := []string{"restore", "--home", w.Gateway.Home}
	if path != "" {
		args = append(args, "--backup", path)
	}
	if accept {
		args = append(args, "--accept-ledger-loss")
	}
	_, err := w.execute(false, w.bridge, args...)
	return err
}
func (w *World) RemoveCommandLedger() error {
	for _, suffix := range []string{"", "-wal", "-shm"} {
		if err := os.Remove(filepath.Join(w.Gateway.Home, "ledger.db") + suffix); err != nil && !os.IsNotExist(err) {
			return err
		}
	}
	return nil
}
func (w *World) CommandLedgerLossBlocked() bool {
	ctx, cancel := context.WithTimeout(w.ctx, 3*time.Second)
	defer cancel()
	_, err := commandOutput(ctx, w.bridge, "start", "--home", w.Gateway.Home, "--api-addr", "127.0.0.1:0", "--peer-addr", "127.0.0.1:0")
	return ctx.Err() == nil && err != nil && strings.Contains(err.Error(), "ledger_missing")
}
