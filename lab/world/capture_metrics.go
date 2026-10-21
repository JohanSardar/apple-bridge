package world

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

type captureMetricsKey struct{}
type CaptureMetrics struct {
	mu     sync.Mutex
	counts map[string]int
}

func WithCaptureMetrics(ctx context.Context) (context.Context, *CaptureMetrics) {
	metrics := &CaptureMetrics{counts: map[string]int{}}
	return context.WithValue(ctx, captureMetricsKey{}, metrics), metrics
}
func (m *CaptureMetrics) Resets() int {
	m.mu.Lock()
	defer m.mu.Unlock()
	total := 0
	for _, count := range m.counts {
		total += count
	}
	return total
}
func (w *World) recordCaptureMetrics() {
	metrics, ok := w.ctx.Value(captureMetricsKey{}).(*CaptureMetrics)
	if !ok {
		return
	}
	endpoints := append([]Endpoint{w.Gateway}, w.Replicas...)
	for _, endpoint := range endpoints {
		path := filepath.Join(endpoint.Home, "lab-capture-observer-resets")
		var raw []byte
		if endpoint.Remote {
			ctx, cancel := context.WithTimeout(context.WithoutCancel(w.ctx), 5*time.Second)
			value, err := commandOutput(ctx, "ssh", "ubuntu", "cat "+quote(path))
			cancel()
			if err == nil {
				raw = []byte(value)
			}
		} else {
			raw, _ = os.ReadFile(path)
		}
		count := strings.Count(string(raw), "\n")
		metrics.mu.Lock()
		metrics.counts[endpoint.Home] = max(metrics.counts[endpoint.Home], count)
		metrics.mu.Unlock()
	}
}
