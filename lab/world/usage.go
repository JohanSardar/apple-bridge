package world

import (
	"context"
	"io/fs"
	"os"
	"path/filepath"
	"strings"
)

func NewUsage(ctx context.Context, level string) (*World, error) {
	setup := NativeSetup{BeforeStart: func(w *World) error { return w.stageUsageTruth() }}
	return newEnvironment(ctx, level, 1, 0, level == "real", "", setup, BlobOptions{Scope: "usage:read,system:read"})
}
func (w *World) UsageTruthRoot() string { return filepath.Join(w.Dir, "usage-truth-biome") }
func (w *World) stageUsageTruth() error {
	source := filepath.Join(os.Getenv("HOME"), "Library", "Biome")
	target := w.UsageTruthRoot()
	stream := filepath.Join("streams", "restricted", "App.InFocus")
	err := filepath.WalkDir(filepath.Join(source, stream), func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.Name() == "tombstone" && entry.IsDir() {
			return filepath.SkipDir
		}
		relative, err := filepath.Rel(source, path)
		if err != nil {
			return err
		}
		destination := filepath.Join(target, relative)
		if entry.IsDir() {
			return os.MkdirAll(destination, 0700)
		}
		numeric := entry.Name() != ""
		for _, ch := range entry.Name() {
			numeric = numeric && ch >= '0' && ch <= '9'
		}
		if !numeric || !entry.Type().IsRegular() {
			return nil
		}
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		return os.WriteFile(destination, data, 0600)
	})
	if err != nil {
		return err
	}
	if err = os.MkdirAll(filepath.Join(target, "sync"), 0700); err != nil {
		return err
	}
	backup := filepath.Join(target, "sync", "sync.db")
	_, err = commandOutput(w.ctx, "/usr/bin/sqlite3", "-readonly", filepath.Join(source, "sync", "sync.db"), ".backup '"+strings.ReplaceAll(backup, "'", "''")+"'")
	if err != nil {
		return err
	}
	return os.Chmod(backup, 0600)
}
