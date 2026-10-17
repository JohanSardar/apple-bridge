package world

import "context"

func NewContacts(ctx context.Context, level string, replicas int, native bool) (*World, error) {
	return newEnvironment(ctx, level, replicas, 0, native, BlobOptions{Scope: "contacts:write,messages:read,system:read"})
}
