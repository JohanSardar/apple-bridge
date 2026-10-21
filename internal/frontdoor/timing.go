package frontdoor

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func invokeTimed(ctx context.Context, deps Deps, principal trust.Principal, descriptor contracts.OperationDescriptor, input json.RawMessage) (any, float64, error) {
	if !descriptor.ReadModel {
		result, err := deps.Operations.Invoke(ctx, principal, descriptor.Name, input)
		return result, 0, err
	}
	started := time.Now()
	result, err := deps.Operations.Invoke(ctx, principal, descriptor.Name, input)
	return result, float64(time.Since(started)) / float64(time.Millisecond), err
}
