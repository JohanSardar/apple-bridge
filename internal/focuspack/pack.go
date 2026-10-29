package focuspack

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/shortcut"
)

const Name = "AB Focus"

var ErrUnqualified = errors.New("Focus status readback is unavailable or mismatched")

type Runner interface {
	Run(context.Context, shortcut.Input) (shortcut.Result, error)
}
type Status struct {
	Focused       *bool   `json:"focused"`
	Name          *string `json:"name"`
	Authorization string  `json:"authorization"`
}
type Pack struct {
	Runner     Runner
	ReadStatus func(context.Context) (Status, error)
}
type SetArgs struct {
	Mode  string `json:"mode"`
	On    bool   `json:"on"`
	Until string `json:"until"`
}

func (p Pack) Set(ctx context.Context, args SetArgs) error {
	if p.Runner == nil || p.ReadStatus == nil || args.Mode != "Do Not Disturb" {
		return ErrUnqualified
	}
	if args.Until != "" {
		until, err := time.Parse(time.RFC3339, args.Until)
		if err != nil || !args.On || !until.After(time.Now()) {
			return ErrUnqualified
		}
	}
	before, err := p.ReadStatus(ctx)
	if err != nil {
		return err
	}
	if before.Authorization != "authorized" || before.Focused == nil {
		return ErrUnqualified
	}
	data, err := json.Marshal(args)
	if err != nil {
		return err
	}
	result, err := p.Runner.Run(ctx, shortcut.Input{Shortcut: Name, Data: data, Timeout: 15 * time.Second})
	if err != nil {
		return err
	}
	if result.TimedOut || result.ExitStatus != 0 {
		return ErrUnqualified
	}
	for i := 0; i < 20; i++ {
		status, err := p.ReadStatus(ctx)
		if err != nil {
			return err
		}
		if status.Authorization == "authorized" && status.Focused != nil && *status.Focused == args.On {
			return nil
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(200 * time.Millisecond):
		}
	}
	return ErrUnqualified
}
