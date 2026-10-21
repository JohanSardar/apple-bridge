package replication

import (
	"context"
	"crypto/x509"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/coder/websocket"
)

func (s *Server) readUpstream(ctx context.Context, conn *websocket.Conn, cert *x509.Certificate, session string) (contracts.FeedFrame, error) {
	for {
		bounded, cancel := context.WithTimeout(ctx, 15*time.Second)
		kind, raw, err := conn.Read(bounded)
		cancel()
		if err != nil {
			return contracts.FeedFrame{}, err
		}
		if kind == websocket.MessageBinary {
			if s.UploadChunk == nil {
				return contracts.FeedFrame{}, contracts.ErrInvalid
			}
			id, response, err := s.UploadChunk(ctx, cert, session, raw)
			if err != nil || id == "" {
				return contracts.FeedFrame{}, contracts.ErrInvalid
			}
			if _, err = send(ctx, conn, contracts.FeedFrame{T: "CommandResponse", ID: id, Response: &response}); err != nil {
				return contracts.FeedFrame{}, err
			}
			continue
		}
		if kind != websocket.MessageText || len(raw) > contracts.MaxFrame {
			return contracts.FeedFrame{}, contracts.ErrInvalid
		}
		var frame contracts.FeedFrame
		if json.Unmarshal(raw, &frame) != nil || frame.T == "" {
			return frame, contracts.ErrInvalid
		}
		return frame, nil
	}
}
