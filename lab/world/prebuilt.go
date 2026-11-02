package world

import (
	"context"
	"fmt"
	"os"
)

type prebuiltKey struct{}

func WithPrebuilt(ctx context.Context) context.Context {
	return context.WithValue(ctx, prebuiltKey{}, true)
}

func prebuilt(ctx context.Context) bool {
	enabled, _ := ctx.Value(prebuiltKey{}).(bool)
	return enabled
}

func requireExecutable(path string) error {
	info, err := os.Stat(path)
	if err != nil {
		return err
	}
	if !info.Mode().IsRegular() || info.Mode().Perm()&0111 == 0 {
		return fmt.Errorf("prebuilt executable required: %s", path)
	}
	return nil
}
