//go:build !lab

package runtime

import (
	"context"
	"net/http"
)

func serveRelayLab(ctx context.Context, home string, handler http.Handler) error { return nil }
