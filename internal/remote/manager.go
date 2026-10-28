package remote

import (
	"context"
	"io"
	"net"
	"net/http"
	"tailscale.com/client/local"
	"time"
)

func (r *Remote) Run(ctx context.Context, web, api, feed http.Handler) {
	r.web = web
	r.api = api
	r.feed = feed
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	var active string
	var enabled bool
	stop := func() {}
	defer func() { stop() }()
	for {
		var cfg Config
		if read(r.home, "config", &cfg) == nil && (cfg.ID != active || cfg.Enabled != enabled) {
			stop()
			active = cfg.ID
			enabled = cfg.Enabled
			var previous Status
			if cfg.Backend == "" && read(r.home, "status", &previous) == nil && previous.ID == cfg.ID {
				cfg.Backend = previous.Backend
			}
			status := Status{ID: cfg.ID, State: "off", Backend: cfg.Backend, TLSTermination: "on this Mac"}
			if !cfg.Enabled {
				if cfg.Backend == "system" {
					if err := cleanupSystem(r.home, &local.Client{}); err != nil {
						status.State = "error"
						status.LastError = "Remote cleanup failed"
					}
				}
				r.publish(status)
			} else {
				status.State = "starting"
				stop = r.start(ctx, cfg)
			}
		}
		select {
		case <-ctx.Done():
			return
		case <-ticker.C:
		}
	}
}

func (r *Remote) start(ctx context.Context, cfg Config) func() {
	runCtx, cancel := context.WithCancel(ctx)
	done := make(chan struct{})
	go func() {
		defer close(done)
		defer cancel()
		conn, err := net.Dial("unix", controlPath(r.home, cfg.ID))
		if err != nil {
			var previous Status
			if read(r.home, "status", &previous) != nil || previous.ID != cfg.ID || previous.State != "on" {
				r.cancelled(cfg)
				return
			}
		} else {
			defer conn.Close()
			r.mu.Lock()
			r.control = conn
			r.controlID = cfg.ID
			r.mu.Unlock()
			defer func() { r.mu.Lock(); r.control = nil; r.controlID = ""; r.mu.Unlock() }()
			go func() {
				io.Copy(io.Discard, conn)
				r.mu.RLock()
				ready := r.status.ID == cfg.ID && r.status.State == "on"
				r.mu.RUnlock()
				if !ready {
					cancel()
				}
			}()
		}
		r.serve(runCtx, cfg)
		r.mu.RLock()
		ready := r.status.ID == cfg.ID && r.status.State == "on"
		r.mu.RUnlock()
		if runCtx.Err() != nil && !ready {
			r.cancelled(cfg)
		}
	}()
	return func() { cancel(); <-done }
}
