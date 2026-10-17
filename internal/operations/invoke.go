package operations

import (
	"context"
	"encoding/json"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) Invoke(ctx context.Context, p trust.Principal, name string, input json.RawMessage) (any, error) {
	d, ok := contracts.Descriptor(name)
	if !ok {
		return nil, contracts.ErrUnsupported
	}
	if d.Kind == contracts.CommandOp {
		var c contracts.Command
		if contracts.DecodeInput(input, &c) != nil {
			return nil, contracts.ErrInvalid
		}
		if c.Op != "" && c.Op != name {
			return nil, contracts.ErrInvalid
		}
		c.Op = name
		if c.ID == "" {
			c.ID = contracts.NewCommandID()
		}
		if s.Target == nil {
			return nil, contracts.ErrUnavailable
		}
		return s.Target.Submit(ctx, p, c)
	}
	if name == "commands.status" || name == "commands.list" {
		var q contracts.CommandQuery
		if contracts.DecodeInput(input, &q) != nil || q.Limit < 0 || q.Limit > 1000 {
			return nil, contracts.ErrInvalid
		}
		if s.Target == nil {
			return nil, contracts.ErrUnavailable
		}
		if name == "commands.list" {
			return s.Target.List(ctx, p, q)
		}
		if !contracts.ValidCommandID(q.ID) {
			return nil, contracts.ErrInvalid
		}
		return s.Target.Status(ctx, p, q)
	}
	var q contracts.ReadQuery
	if contracts.DecodeInput(input, &q) != nil {
		return nil, contracts.ErrInvalid
	}
	result, err := s.Read(ctx, p, name, q)

	return result, err
}
