package world

import (
	"os"
	"path/filepath"
	"strconv"
	"time"
)

func (w *World) SeedMessages(count int) error {
	previous := w.Control.Timeout
	w.Control.Timeout = time.Minute
	defer func() { w.Control.Timeout = previous }()
	return w.ControlAction("/seed", map[string]any{"count": count, "collection": "messages.message"})
}
func (w *World) Resources() (map[string]float64, error) {
	result := map[string]float64{}
	for _, endpoint := range append([]Endpoint{w.Gateway}, w.Replicas...) {
		role := "gateway"
		if endpoint.Home != w.Gateway.Home {
			role = "replica"
		}
		info, err := os.Stat(filepath.Join(endpoint.Home, "state.db-wal"))
		if err != nil {
			return nil, err
		}
		result[role+"_wal_bytes"] = float64(info.Size())
		for _, p := range w.processes {
			if p != nil && !p.stopped && p.home == endpoint.Home {
				output, err := commandOutput(w.ctx, "ps", "-o", "rss=", "-p", strconv.Itoa(p.command.Process.Pid))
				if err != nil {
					return nil, err
				}
				rss, err := strconv.ParseFloat(output, 64)
				if err != nil {
					return nil, err
				}
				result[role+"_rss_bytes"] = rss * 1024
			}
		}
	}
	return result, nil
}
