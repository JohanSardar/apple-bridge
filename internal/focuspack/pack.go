package focuspack

import (
	"context"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

const Name = "AB Focus"

var ErrUnqualified = errors.New("Focus pack actions and structured readback are not qualified")

type Runner interface {
	Run(context.Context, shortcut.Input) (shortcut.Result, error)
}

type Pack struct{ Runner Runner }

func (p Pack) Inspect(ctx context.Context) ([]byte, error) {
	runner := p.Runner
	if runner == nil {
		return nil, ErrUnqualified
	}
	result, err := runner.Run(ctx, shortcut.Input{Shortcut: Name, Timeout: 15 * time.Second})
	if err != nil {
		return nil, err
	}
	if result.TimedOut || result.ExitStatus != 0 || len(result.Data) > 32<<10 {
		return nil, ErrUnqualified
	}
	return result.Data, nil
}

type SetArgs struct {
	Mode  string `json:"mode"`
	Until string `json:"until,omitempty"`
}

func (p Pack) Set(ctx context.Context, args SetArgs) error {
	return ErrUnqualified
}
