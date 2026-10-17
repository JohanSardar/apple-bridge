//go:build !lab

package replication

import "context"

func commandApplyDelay(context.Context) error { return nil }
