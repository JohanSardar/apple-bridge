package frontdoor

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func reminderReadModel(name string) bool {
	return name == "reminders.lists" || name == "reminders.items"
}

func invokeTimed(ctx context.Context, deps Deps, principal trust.Principal, name string, input json.RawMessage) (any, error, float64) {
	if !reminderReadModel(name) {
		result, err := deps.Operations.Invoke(ctx, principal, name, input)
		return result, err, 0
	}
	started := time.Now()
	result, err := deps.Operations.Invoke(ctx, principal, name, input)
	return result, err, float64(time.Since(started)) / float64(time.Millisecond)
}
