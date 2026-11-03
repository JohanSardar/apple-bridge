package scenarios

import (
	"encoding/json"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"slices"
	"strings"
)

var gatewayProductImport = regexp.MustCompile(`"github.com/JohanSardar/apple-bridge/(internal/[^"\n]+)"`)
var gatewaySemanticCall = regexp.MustCompile(`contracts\.(CommandHash|Descriptor|GrantsForSource|IPhoneCompletionEvidence|MacCompletionEvidence|MacDescriptors|NormalizeContactHandle|OperationSchema|ValidateCommand|ValidateIPhoneCommand|BlobReferences)\(`)

func portableGatewaySimScenario(s Scenario) bool {
	switch s.ID {
	case "RELAY-ROTATE-GATED", "SEC-REMOTE-ACCESS", "SC-RUN-REAL", "CORE-STALE-FENCE", "CORE-ACK-DURABLE":
		return true
	}
	raw, err := os.ReadFile("lab/test-selection.json")
	if err != nil {
		return false
	}
	var inventory struct {
		Scenarios []struct {
			ID       string   `json:"id"`
			Host     string   `json:"host"`
			PClass   *bool    `json:"p_class"`
			Levels   []string `json:"levels"`
			Paths    []string `json:"code_paths"`
			Semantic []string `json:"semantic_contract_calls"`
		} `json:"scenarios"`
	}
	if json.Unmarshal(raw, &inventory) != nil {
		return false
	}
	for _, entry := range inventory.Scenarios {
		if entry.ID != s.ID {
			continue
		}
		if entry.Host != runtime.GOOS || entry.PClass == nil || *entry.PClass && runtime.GOOS != "darwin" || !slices.Contains(entry.Levels, "sim") || len(entry.Paths) == 0 || len(entry.Semantic) > 0 {
			return false
		}
		for _, path := range entry.Paths {
			if filepath.Clean(path) != path || !strings.HasPrefix(path, "lab/scenarios/") || filepath.Ext(path) != ".go" {
				return false
			}
			source, err := os.ReadFile(path)
			if err != nil || gatewaySemanticCall.Match(source) {
				return false
			}
			for _, match := range gatewayProductImport.FindAllSubmatch(source, -1) {
				if string(match[1]) != "internal/contracts" {
					return false
				}
			}
		}
		return true
	}
	return false
}
