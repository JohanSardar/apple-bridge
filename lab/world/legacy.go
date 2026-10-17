package world

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
)

const LegacyRevision = "f0cea78"

func BuildLegacy(ctx context.Context) error {
	if err := os.MkdirAll("/tmp/ablab", 0700); err != nil {
		return err
	}
	directory, err := os.MkdirTemp("/tmp/ablab", "legacy-build-*")
	if err != nil {
		return err
	}
	defer os.RemoveAll(directory)
	archive := exec.CommandContext(ctx, "git", "archive", LegacyRevision)
	extract := exec.CommandContext(ctx, "tar", "-x", "-C", directory)
	pipe, err := archive.StdoutPipe()
	if err != nil {
		return err
	}
	extract.Stdin = pipe
	if err = extract.Start(); err != nil {
		return err
	}
	if err = archive.Run(); err != nil {
		extract.Process.Kill()
		extract.Wait()
		return err
	}
	if err = extract.Wait(); err != nil {
		return err
	}
	outputPath, err := filepath.Abs("bin/bridge-feed1")
	if err != nil {
		return err
	}
	build := exec.CommandContext(ctx, "go", "build", "-p", "2", "-o", outputPath, "./cmd/bridge")
	build.Dir = directory
	output, err := build.CombinedOutput()
	if err != nil {
		return fmt.Errorf("legacy build: %w: %s", err, output)
	}
	return nil
}
func NewLegacy(ctx context.Context, level string, replicas, seed int) (*World, error) {
	root, err := os.Getwd()
	if err != nil {
		return nil, err
	}
	return newWorld(ctx, level, replicas, seed, filepath.Join(root, "bin", "bridge-feed1"))
}
