package remote

import (
	"context"
	"fmt"
	"net"
	"net/http"
	"time"
)

func (r *Remote) ready(ctx context.Context, status *Status, listener net.Listener, proxyPath string, fail func(error)) {
	status.State = "probing"
	status.PendingLink = ""
	status.Message = "Checking public HTTPS, OAuth, MCP and 60 second SSE"
	r.publish(*status)
	serving, cancel := context.WithCancel(ctx)
	defer cancel()
	serverErrors := make(chan error, 1)
	done := make(chan struct{})
	go func() {
		defer close(done)
		r.serveListener(serving, listener, proxyPath, func(err error) { serverErrors <- err; cancel() })
	}()
	defer func() { cancel(); <-done }()
	probe := r.options.Probe
	if probe == nil {
		probe = publicProbe
	}
	bounded, stop := context.WithTimeout(serving, 2*time.Minute)
	expiry, err := probe(bounded, status.URL)
	stop()
	if err != nil {
		fail(err)
		return
	}
	if ctx.Err() != nil {
		return
	}
	status.CertificateExpiry = expiry
	status.State = "on"
	status.Message = ""
	r.publish(*status)
	<-serving.Done()
	select {
	case err := <-serverErrors:
		fail(err)
	default:
	}
}

func streamCheck(w http.ResponseWriter, req *http.Request) {
	if req.Method != http.MethodGet {
		w.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-store")
	ticker := time.NewTicker(time.Second)
	defer ticker.Stop()
	deadline := time.NewTimer(75 * time.Second)
	defer deadline.Stop()
	for {
		if _, err := fmt.Fprint(w, ": bridge remote check\n\n"); err != nil {
			return
		}
		if err := http.NewResponseController(w).Flush(); err != nil {
			return
		}
		select {
		case <-req.Context().Done():
			return
		case <-deadline.C:
			return
		case <-ticker.C:
		}
	}
}
