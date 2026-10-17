package world

import "context"

func NewContacts(ctx context.Context, level string, replicas int, native bool, includeMessages bool) (*World, error) {
	scope := "contacts:write,system:read"
	if includeMessages {
		scope += ",messages:read"
	}
	return newEnvironment(ctx, level, replicas, 0, native, BlobOptions{Scope: scope})
}
