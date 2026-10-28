package remote

import (
	"net"
	"net/http"
	"strings"
	"sync"
)

type Remote struct {
	options   Options
	control   net.Conn
	controlID string
	home      string
	mu        sync.RWMutex
	status    Status
	web       http.Handler
	api       http.Handler
	feed      http.Handler
}

func New(home string) *Remote { return &Remote{home: home} }
func (r *Remote) Host() string {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return strings.TrimPrefix(r.status.URL, "https://")
}

func (r *Remote) publish(status Status) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.status = status
	write(r.home, "status", status)
}
