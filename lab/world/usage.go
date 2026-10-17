package world

import "context"

func NewUsage(ctx context.Context, level string) (*World, error) {
	return newEnvironment(ctx, level, 1, 0, level == "real", "", NativeSetup{}, BlobOptions{Scope: "usage:read,system:read"})
}
