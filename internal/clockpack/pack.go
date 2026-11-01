package clockpack

import (
	"context"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

const Name = "AB Clock"

var ErrUnqualified = errors.New("Clock pack structured readback is not qualified")

type Runner interface {
	Run(context.Context, shortcut.Input) (shortcut.Result, error)
}

type Pack struct {
	Runner  Runner
	Timeout time.Duration
}

func (p Pack) Inspect(ctx context.Context) ([]byte, error) {
	runner := p.Runner
	if runner == nil {
		runner = shortcut.CLI{}
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

func (p Pack) Alarms(ctx context.Context) error {
	if _, err := p.Inspect(ctx); err != nil {
		return err
	}
	return ErrUnqualified
}
