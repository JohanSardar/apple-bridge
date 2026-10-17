package scenarios

import (
	"context"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runCommands(ctx context.Context, w *world.World, id string, e *Evidence) {
	status, err := world.Request(ctx, w.Gateway, "/v1/op/reminders.create", map[string]any{"command_id": "cmd-test", "args": map[string]any{"title": "[ab-test] command"}}, nil)
	e.Assertions = append(e.Assertions, Assertion{Name: "command_admitted", Passed: err == nil && status == 200})
}
