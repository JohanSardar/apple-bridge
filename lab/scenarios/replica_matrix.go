package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func ReplicaMatrix(ctx context.Context, paths []string, asJSON bool, jobs int) error {
	scenarios, err := loadLevel(paths, "replica")
	if err != nil {
		return err
	}
	selection := world.ImplementationSelection(ctx)
	blocked := false
	for _, impl := range append([]string{selection.Gateway, selection.Replica}, selection.Replicas...) {
		if _, err = selection.Binary(impl, "bin/bridge"); err != nil {
			for _, scenario := range scenarios {
				if slices.Contains(scenario.Levels, "replica") {
					fmt.Fprintf(os.Stderr, "BLOCKED %s: %s; missing executable blocks required interfaces: %s\n", scenario.ID, err, replicaRequirements(scenario))
					if reason := replicaQualificationBlocker(scenario); reason != "" {
						fmt.Fprintf(os.Stderr, "BLOCKED %s: %s\n", scenario.ID, reason)
					}
				}
			}
			fmt.Fprintf(os.Stderr, "BLOCKED REPLICA-MIXED-CONVERGENCE: %s\n", err)
			blocked = true
			break
		}
	}
	if blocked {
		return fmt.Errorf("replica matrix blocked; no Rust qualification evidence produced")
	}
	for _, scenario := range scenarios {
		if reason := replicaQualificationBlocker(scenario); reason != "" {
			fmt.Fprintf(os.Stderr, "BLOCKED %s: %s\n", scenario.ID, reason)
			blocked = true
		}
	}
	suiteErr := replicaMatrixSuite(ctx, scenarios, asJSON, jobs)
	mixed := selection
	mixed.Replicas = []string{"go", "rust"}
	if selection.Replica == "go" && selection.Gateway == "go" {
		mixed.Replicas = []string{"go", "go"}
	}
	bounded, cancel := context.WithTimeout(world.WithImplementations(ctx, mixed), 2*time.Minute)
	defer cancel()
	impl := "mixed"
	if mixed.Gateway == "go" && mixed.Replica == "go" {
		impl = "go"
	}
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{GitSHA: strings.TrimSpace(string(sha)), Scenario: "REPLICA-MIXED-CONVERGENCE", Level: "replica", RunID: contracts.NewID("matrix"), At: time.Now().UTC(), Implementations: mixed, Impl: impl, Metrics: map[string]any{}}
	w, err := world.New(bounded, "replica", 2, 100)
	e.Assertions = append(e.Assertions, Assertion{Name: "selected_binaries_started", Passed: err == nil, Detail: fmt.Sprint(err)})
	if err == nil {
		defer w.Close()
		err = w.Await(30*time.Second, func() bool { return convergence(w) })
		e.Assertions = append(e.Assertions, Assertion{Name: "mixed_convergence", Passed: err == nil})
		var previous string
		for i, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
			records, cursor, readErr := w.RecordsAt(endpoint)
			digest := snapshotDigest(records)
			e.Metrics[fmt.Sprintf("snapshot_%d", i)] = map[string]any{"impl": endpoint.Impl, "digest": digest, "count": len(records), "cursor": cursor}
			e.Assertions = append(e.Assertions, Assertion{Name: fmt.Sprintf("snapshot_%d_identical", i), Passed: readErr == nil && len(records) > 0 && (i == 0 || previous == digest)})
			previous = digest
		}
	}
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	if err = Save(e); err != nil {
		return err
	}
	if asJSON {
		if err := json.NewEncoder(os.Stdout).Encode(e); err != nil {
			return err
		}
	}
	fmt.Fprintf(os.Stderr, "%s passed=%t\n", e.Scenario, e.Passed)
	if suiteErr != nil {
		return suiteErr
	}
	if !e.Passed {
		return fmt.Errorf("mixed replica convergence failed")
	}
	if blocked {
		return fmt.Errorf("replica matrix incomplete: harness blockers remain; runnable scenarios and convergence executed")
	}
	return nil
}

func snapshotDigest(records []contracts.Record) string {
	ordered := slices.Clone(records)
	slices.SortFunc(ordered, func(a, b contracts.Record) int { return strings.Compare(a.Collection+"/"+a.ID, b.Collection+"/"+b.ID) })
	hash := sha256.New()
	for _, record := range ordered {
		fmt.Fprintf(hash, "%s\x00%s\x00%d\n", record.Collection, record.ID, record.Version)
	}
	return hex.EncodeToString(hash.Sum(nil))
}
