package scenarios

import (
	"context"
	"os/exec"
	"runtime"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func runRelayCertificates(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.CommandContext(ctx, "git", "rev-parse", "HEAD").Output()
	e := Evidence{Impl: "rust", RunID: contracts.NewID("run"), GitSHA: strings.TrimSpace(string(sha)), Scenario: s.ID, Level: level, At: time.Now().UTC(), Hosts: []string{"build"}, NetworkPath: "offline fixture adapters only", Conditions: map[string]string{"ca": "RFC 8657 enforcing fixture", "dns": "in-memory transaction adapter", "ct": "injected public-search fixture"}}
	if level != "sim" || runtime.GOOS != "linux" {
		e.Assertions = append(e.Assertions, Assertion{Name: "linux_sim_required", Passed: false})
		return e
	}
	crate, test := "ab-remote", "relay_ct_independent"
	expected := []string{"fake_all_clear_ignored", "unexpected_wildcard", "unexpected_precertificate", "aggregator_outage_unknown", "aggregator_lag_unknown", "stale_monitor_unknown", "doctor_and_push_hook", "all_owned_names", "precertificate_der"}
	if s.ID == "RELAY-CAA-ACCOUNT" {
		crate, test = "ab-relay-control", "relay_caa_account"
		expected = []string{"enrollment_caa_records", "wrong_account_denied", "wrong_method_denied", "alternate_issuer_denied", "wildcard_denied", "signed_rotation_recovery", "cname_target_policy", "dns_operator_residual_risk", "dns_failure_preserves_binding"}
	}
	bounded, cancel := context.WithTimeout(ctx, 5*time.Minute)
	defer cancel()
	command := exec.CommandContext(bounded, "cargo", "test", "--manifest-path", "rust/Cargo.toml", "--offline", "--locked", "-p", crate, "--features", "r2-certificates", "--test", test, "--", "--nocapture")
	output, err := command.CombinedOutput()
	seen := map[string]bool{}
	for _, line := range strings.Split(string(output), "\n") {
		if name, ok := strings.CutPrefix(strings.TrimSpace(line), "R2_ASSERT\t"); ok {
			seen[name] = true
		}
	}
	detail := ""
	if err != nil {
		detail = string(output)
		if len(detail) > 8192 {
			detail = detail[len(detail)-8192:]
		}
	}
	e.Assertions = append(e.Assertions, Assertion{Name: "rust_fixture_executed", Passed: err == nil, Detail: detail})
	e.Passed = err == nil
	for _, name := range expected {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: seen[name]})
		e.Passed = e.Passed && seen[name]
	}
	return e
}
