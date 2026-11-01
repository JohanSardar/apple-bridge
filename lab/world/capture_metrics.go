package world

import (
	"context"
	"os"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"sync"
	"time"
)

type captureMetricsKey struct{}
type CaptureMetrics struct {
	mu         sync.Mutex
	counts     map[string]int
	helperPIDs map[string]bool
}

func WithCaptureMetrics(ctx context.Context) (context.Context, *CaptureMetrics) {
	metrics := &CaptureMetrics{counts: map[string]int{}, helperPIDs: map[string]bool{}}
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
func (m *CaptureMetrics) HelperPIDs() []string {
	m.mu.Lock()
	defer m.mu.Unlock()
	result := make([]string, 0, len(m.helperPIDs))
	for pid := range m.helperPIDs {
		result = append(result, pid)
	}
	sort.Strings(result)
	return result
}

func (w *World) recordCaptureMetrics() {
	metrics, ok := w.ctx.Value(captureMetricsKey{}).(*CaptureMetrics)
	if !ok {
		return
	}
	if w.Native {
		raw, _ := os.ReadFile(filepath.Join(w.Gateway.Home, "helper", "helper.pid"))
		pid := strings.TrimSpace(string(raw))
		if value, err := strconv.Atoi(pid); err == nil && value > 0 {
			metrics.mu.Lock()
			metrics.helperPIDs[pid] = true
			metrics.mu.Unlock()
		}
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
