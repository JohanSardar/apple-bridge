package remote

import (
	"context"
	"net/http"
	"tailscale.com/client/local"
	"time"
)

func (r *Remote) Run(ctx context.Context, web, api http.Handler) {
	r.web = web
	r.api = api
	ticker := time.NewTicker(100 * time.Millisecond)
	defer ticker.Stop()
	var active string
	stop := func() {}
	defer func() { stop() }()
	for {
		var cfg Config
		if read(r.home, "config", &cfg) == nil && cfg.ID != active {
			stop()
			active = cfg.ID
			var previous Status
			if cfg.Backend == "" && read(r.home, "status", &previous) == nil && previous.ID == cfg.ID {
				cfg.Backend = previous.Backend
			}
			status := Status{ID: cfg.ID, State: "off", Backend: cfg.Backend}
			if !cfg.Enabled {
				if err := cleanupSystem(r.home, &local.Client{}); err != nil {
					status.State = "error"
					status.LastError = err.Error()
				}
				r.publish(status)
			} else {
				status.State = "starting"
				r.publish(status)
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
	go func() { defer close(done); defer cancel(); r.serve(runCtx, cfg) }()
	return func() { cancel(); <-done }
}
