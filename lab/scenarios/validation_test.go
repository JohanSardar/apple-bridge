package scenarios

import (
	"os"
	"path/filepath"
	"testing"
)

func TestScenarioDecoderRejectsMalformedOptions(t *testing.T) {
	for name, document := range map[string]string{
		"unknown field":     "typo: true\nthen: [{converge: {}}]",
		"unknown action":    "when: [{typo: {}}]",
		"unknown option":    "then: [{latency: {p95_mss: 1000}}]",
		"string integer":    "then: [{latency: {p95_ms: impossible}}]",
		"fraction integer":  "then: [{latency: {p95_ms: 1.5}}]",
		"string boolean":    "then: [{resumed: {without_snapshot: impossible}}]",
		"multiple actions":  "then: [{converge: {}, source_ready: {}}]",
		"missing options":   "then: [{latency: null}]",
		"zero rate":         "when: [{changes: {per_second: 0}}]",
		"trailing document": "then: [{converge: {}}]\n---\n{}",
	} {
		t.Run(name, func(t *testing.T) {
			file := filepath.Join(t.TempDir(), "invalid.yaml")
			if err := os.WriteFile(file, []byte("id: INVALID\nfeature: core.harness\nlevels: [sim]\n"+document+"\n"), 0600); err != nil {
				t.Fatal(err)
			}
			if _, err := Load([]string{file}); err == nil {
				t.Fatal("malformed scenario loaded")
			}
		})
	}
}
