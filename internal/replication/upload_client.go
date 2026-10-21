package replication

import (
	"context"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func (c *Client) UploadChunk(ctx context.Context, id, digest string, offset int64, data []byte) (contracts.CommandResponse, error) {
	if !contracts.ValidCommandID(id) {
		return contracts.CommandResponse{}, contracts.ErrInvalid
	}
	raw, err := contracts.EncodeBlobChunk(digest, offset, data)
	if err != nil {
		return contracts.CommandResponse{}, err
	}
	return c.exchange(ctx, id, func(bounded context.Context, session *commandSession) error {
		return writeFrame(bounded, session.conn, websocket.MessageBinary, raw, true)
	})
}
