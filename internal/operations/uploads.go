package operations

import (
	"context"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (g *LocalGateway) borrowUploads(ctx context.Context, owner string, c contracts.Command) (map[string]contracts.BlobReference, func(), error) {
	if !strings.HasPrefix(c.Op, "messages.") {
		return nil, func() {}, nil
	}
	args, err := contracts.ValidateMessagesCommand(c)
	if err != nil {
		return nil, nil, err
	}
	if len(args.Files) == 0 {
		return nil, func() {}, nil
	}
	if g.Uploads == nil {
		return nil, nil, contracts.ErrUnavailable
	}
	return g.Uploads.BorrowMessageUploads(ctx, owner, args.Files)
}
