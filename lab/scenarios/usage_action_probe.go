package scenarios

import (
	"context"
)

func runUsageActionProbe(ctx context.Context, s Scenario, level string) Evidence {
	return runUsageData(ctx, s, level)
}
