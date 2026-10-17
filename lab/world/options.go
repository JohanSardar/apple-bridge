package world

import "context"

type Options struct{ DirectTailscale bool }
type optionsKey struct{}

func WithOptions(ctx context.Context, options Options) context.Context {
	return context.WithValue(ctx, optionsKey{}, options)
}
