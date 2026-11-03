//go:build !lab

package remote

import (
	"context"
	"tailscale.com/client/local"
	"time"
)

func installedLocalClient() *local.Client { return &local.Client{} }
func installedProbe(ctx context.Context, origin string) (time.Time, error) {
	return publicProbe(ctx, origin)
}

func installedSetupFailure(error) {}
