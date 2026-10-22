package world

import "context"

func NewContacts(ctx context.Context, level string, replicas int, native bool, includeMessages bool, setups ...NativeSetup) (*World, error) {
	scope := "contacts:write,system:read"
	if includeMessages {
		scope += ",messages:read"
	}
	setup := NativeSetup{}
	if len(setups) > 0 {
		setup = setups[0]
	}
	return newEnvironment(ctx, level, replicas, 0, native, "", setup, BlobOptions{Scope: scope})
}
