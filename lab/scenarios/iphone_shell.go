package scenarios

import (
	"context"
	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

func runIPhoneShell(ctx context.Context, s Scenario, level string) Evidence {
	e := Evidence{Scenario: s.ID, Level: level, At: time.Now().UTC(), Passed: true}
	add := func(name string, ok bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok})
		e.Passed = e.Passed && ok
	}
	dir, err := os.MkdirTemp("", "ab-test-shell-")
	add("isolated_build", err == nil)
	if err != nil {
		return e
	}
	defer os.RemoveAll(dir)
	binary := filepath.Join(dir, "shell-proof")
	cmd := exec.CommandContext(ctx, "xcrun", "swiftc", "-swift-version", "6", "-warnings-as-errors", "-parse-as-library", "ios/Delivery/PhoneJob.swift", "ios/Capabilities/Approvals/ApprovalInbox.swift", "ios/Pairing/PairingState.swift", "ios/App/ShellModel.swift", "lab/ios/ShellProof.swift", "-o", binary)
	output, err := cmd.CombinedOutput()
	add("shell_view_models_build", err == nil)
	if err != nil {
		return e
	}
	output, err = exec.CommandContext(ctx, binary).CombinedOutput()
	add("shell_view_models_execute", err == nil)
	var checks map[string]bool
	add("shell_proof_decoded", json.Unmarshal(output, &checks) == nil)
	for _, name := range []string{"tab_structure", "unpaired_status", "unpaired_no_request", "pair_failure", "paired_status", "connecting_state", "connected_status", "offline_status", "reconnect_status", "failed_repair_keeps_identity", "status_recovers_after_error", "existing_pair_status", "approval_order_and_expiry", "approval_list_rendering", "approval_empty_state", "approval_expiry_refresh", "approval_removal_refresh", "stored_approval_rendering", "decided_approval_removed"} {
		add(name, checks[name])
	}
	return e
}
