//go:build !lab

package operations

import "context"

func uploadForwardCheckpoint(context.Context, string, int64, int64) error { return nil }
