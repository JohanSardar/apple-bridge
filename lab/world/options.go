package world

import "context"

type Options struct {
	GatewayDials             bool
	DirectTailscale          bool
	PersistentReplicaStorage bool
}
type optionsKey struct{}

func WithOptions(ctx context.Context, options Options) context.Context {
	return context.WithValue(ctx, optionsKey{}, options)
}

func WithPersistentReplicaStorage(ctx context.Context) context.Context {
	options, _ := ctx.Value(optionsKey{}).(Options)
	options.PersistentReplicaStorage = true
	return WithOptions(ctx, options)
}

func WithGatewayDials(ctx context.Context) context.Context {
	options, _ := ctx.Value(optionsKey{}).(Options)
	options.GatewayDials = true
	return WithOptions(ctx, options)
}
