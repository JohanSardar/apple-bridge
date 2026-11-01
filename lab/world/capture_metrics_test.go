package world

import (
	"context"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

func TestCaptureMetricsOwnedHelperProcesses(t *testing.T) {
	ctx, metrics := WithCaptureMetrics(context.Background())
	home := t.TempDir()
	if err := os.MkdirAll(filepath.Join(home, "helper"), 0700); err != nil {
		t.Fatal(err)
	}
	w := &World{ctx: ctx, Native: true, Gateway: Endpoint{Home: home}}
	for _, pid := range []string{"123", "456", "0", "not a pid", "456"} {
		if err := os.WriteFile(filepath.Join(home, "helper", "helper.pid"), []byte(pid), 0600); err != nil {
			t.Fatal(err)
		}
		w.recordCaptureMetrics()
	}
	if got := metrics.HelperPIDs(); !reflect.DeepEqual(got, []string{"123", "456"}) {
		t.Fatalf("owned helper processes: %v", got)
	}
	metrics.HelperPIDs()[0] = "changed"
	if got := metrics.HelperPIDs(); got[0] != "123" {
		t.Fatalf("caller mutated captured ownership: %v", got)
	}
}
