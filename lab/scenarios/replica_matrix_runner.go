package scenarios

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/lab/world"
	"gopkg.in/yaml.v3"
)

func replicaMatrixSuite(ctx context.Context, scenarios []Scenario, asJSON bool, jobs int) error {
	if err := world.Build(ctx, "replica"); err != nil {
		return err
	}
	directory, err := os.MkdirTemp("", "ab-replica-matrix-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(directory)
	executable, err := os.Executable()
	if err != nil {
		return err
	}
	selection := world.ImplementationSelection(ctx)
	pending := make(chan Scenario)
	results := make(chan error, len(scenarios))
	var workers sync.WaitGroup
	var resources sync.RWMutex
	for range jobs {
		workers.Add(1)
		go func() {
			defer workers.Done()
			for scenario := range pending {
				raw, err := yaml.Marshal(scenario)
				if err != nil {
					results <- err
					continue
				}
				file, err := os.CreateTemp(directory, "scenario-*.yaml")
				if err != nil {
					results <- err
					continue
				}
				_, writeErr := file.Write(raw)
				closeErr := file.Close()
				if writeErr != nil {
					results <- writeErr
					continue
				}
				if closeErr != nil {
					results <- closeErr
					continue
				}
				args := []string{"run", file.Name(), "--level", "replica", "--prebuilt", "--impl", "gateway=" + selection.Gateway + ",replica=" + selection.Replica}
				if selection.LocalReplicas {
					args = append(args, "--local-replicas")
				}
				if selection.RustBinary != "" {
					args = append(args, "--rust-binary", selection.RustBinary)
				}
				if asJSON {
					args = append(args, "--json")
				}
				logPath := filepath.Join(directory, filepath.Base(file.Name())+".log")
				log, err := os.Create(logPath)
				if err != nil {
					results <- err
					continue
				}
				cmd := exec.CommandContext(ctx, executable, args...)
				errorLog, err := os.Create(logPath + ".stderr")
				if err != nil {
					log.Close()
					results <- err
					continue
				}
				cmd.Stdout, cmd.Stderr = log, errorLog
				cmd.Cancel = func() error { return cmd.Process.Signal(os.Interrupt) }
				cmd.WaitDelay = 30 * time.Second
				if latencyCritical(scenario) || scenario.ID == "CORE-QUERY-REPLICA" {
					resources.Lock()
					err = cmd.Run()
					resources.Unlock()
				} else {
					resources.RLock()
					err = cmd.Run()
					resources.RUnlock()
				}
				log.Close()
				errorLog.Close()
				output, readErr := os.ReadFile(logPath)
				fmt.Fprint(os.Stdout, string(output))
				diagnostic, diagnosticErr := os.ReadFile(logPath + ".stderr")
				fmt.Fprint(os.Stderr, string(diagnostic))
				if readErr == nil {
					readErr = diagnosticErr
				}
				if err != nil {
					results <- fmt.Errorf("%s: %w", scenario.ID, err)
				} else {
					results <- readErr
				}
			}
		}()
	}
	for _, scenario := range scenarios {
		if replicaQualificationBlocker(scenario) == "" {
			pending <- scenario
		}
	}
	close(pending)
	workers.Wait()
	close(results)
	failed, ran := 0, 0
	for err := range results {
		ran++
		if err != nil {
			failed++
			fmt.Fprintln(os.Stderr, err)
		}
	}
	fmt.Fprintf(os.Stderr, "replica scenarios: runnable=%d failed=%d blocked=%d total=%d\n", ran, failed, len(scenarios)-ran, len(scenarios))
	if failed > 0 {
		return fmt.Errorf("%d replica scenarios failed", failed)
	}
	return nil
}
