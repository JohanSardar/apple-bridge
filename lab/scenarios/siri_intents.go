package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

func validBridgeStatus(data []byte) bool {
	switch strings.TrimSpace(string(data)) {
	case "Apple Bridge background service is enabled.",
		"Apple Bridge background service needs approval in System Settings.",
		"Apple Bridge background service is not registered.",
		"Apple Bridge background service configuration was not found.",
		"Apple Bridge background service status is unknown.":
		return true
	}
	return false
}

func runSiriIntents(ctx context.Context, s Scenario, level string) Evidence {
	var cli shortcutProbeCLI = shortcut.CLI{}
	qualified := false
	metadata, metadataErr := os.ReadFile("packaging/app-intents/Metadata.appintents/extract.actionsdata")
	source, sourceErr := os.ReadFile("native/Sources/AppleBridgeHelper/BridgeStatusIntent.swift")
	digest, digestErr := os.ReadFile("packaging/app-intents/source.sha256")
	var actions struct {
		Actions map[string]struct {
			Identifier string `json:"identifier"`
		} `json:"actions"`
	}
	parsed := json.Unmarshal(metadata, &actions) == nil && actions.Actions["BridgeStatusIntent"].Identifier == "BridgeStatusIntent"
	emitted := parsed && metadataErr == nil && sourceErr == nil && digestErr == nil && strings.TrimSpace(string(digest)) == fmt.Sprintf("%x", sha256.Sum256(source))
	if level == "sim" {
		cli = &shortcutProbeSimulation{entries: []shortcut.Entry{{Name: "AB Bridge Status"}}, result: shortcut.Result{Data: []byte("Apple Bridge background service is not registered.")}}
		qualified = emitted
	} else {
		artifact, err := os.ReadFile("packaging/shortcuts/AB Bridge Status.shortcut")
		qualified = emitted && err == nil && len(artifact) >= 4 && string(artifact[:4]) == "AEA1"
	}
	e := runShortcutProbe(ctx, s, level, "AB Bridge Status", cli, qualified, validBridgeStatus)
	e.Assertions = append(e.Assertions, Assertion{Name: "status_intent_metadata_matches_source", Passed: emitted})
	e.Passed = e.Passed && emitted
	return e
}
