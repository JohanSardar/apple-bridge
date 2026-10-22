//go:build !lab

package replication

import "context"

func captureFeatures() []string { return []string{"capture_atomic"} }

func capturePartBoundary(context.Context) error { return nil }
