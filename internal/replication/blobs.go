package replication

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
	"io"
	"time"
)

func (s *Server) serveBlobs(ctx context.Context, conn *websocket.Conn, p trust.Principal, wants <-chan contracts.FeedFrame) {
	for {
		select {
		case <-ctx.Done():
			return
		case want := <-wants:
			if s.Blobs == nil || !contracts.ValidDigest(want.Digest) || want.Offset < 0 {
				send(ctx, conn, contracts.FeedFrame{T: "BlobUnavailable", Digest: want.Digest, Reason: "unavailable"})
				continue
			}
			grant, err := s.Store.BlobGrant(ctx, p.Visibility(), want.Digest)
			if err != nil {
				send(ctx, conn, contracts.FeedFrame{T: "BlobUnavailable", Digest: want.Digest, Reason: "unauthorized"})
				continue
			}
			f, err := s.Blobs.OpenAuthorized(ctx, grant, want.Digest)
			if err != nil {
				reason := "missing"
				if s.Blobs.Progress().Entries[want.Digest].State == "offloaded" {
					reason = "offloaded"
				}
				send(ctx, conn, contracts.FeedFrame{T: "BlobUnavailable", Digest: want.Digest, Reason: reason})
				continue
			}
			func() {
				defer f.Close()
				if _, err = f.Seek(want.Offset, io.SeekStart); err != nil {
					return
				}
				offset := want.Offset
				buffer := make([]byte, contracts.BlobChunkSize)
				for {
					if ctx.Err() != nil {
						return
					}
					n, readErr := f.Read(buffer)
					if n > 0 {
						raw, err := contracts.EncodeBlobChunk(want.Digest, offset, buffer[:n])
						if err != nil {
							return
						}
						mutateBlobChunk(raw)
						bounded, cancel := context.WithTimeout(ctx, 10*time.Second)
						err = writeFrame(bounded, conn, websocket.MessageBinary, raw, true)
						cancel()
						if err != nil {
							return
						}
						offset += int64(n)
					}
					if readErr == io.EOF {
						send(ctx, conn, contracts.FeedFrame{T: "BlobDone", Digest: want.Digest})
						return
					}
					if readErr != nil {
						return
					}
				}
			}()
		}
	}
}

type blobEvent struct {
	digest string
	offset int64
	bytes  []byte
	done   bool
	reason string
}
type feedSource struct {
	conn    *websocket.Conn
	events  <-chan blobEvent
	started bool
}

func (s *feedSource) Read(ctx context.Context, d string, offset, length int64) ([]byte, bool, error) {
	if !s.started {
		if _, err := send(ctx, s.conn, contracts.FeedFrame{T: "BlobWant", Digest: d, Offset: offset}); err != nil {
			return nil, false, err
		}
		s.started = true
	}
	select {
	case <-ctx.Done():
		return nil, false, ctx.Err()
	case e := <-s.events:
		if e.digest != d {
			return nil, false, contracts.ErrCorrupt
		}
		if e.done {
			s.started = false
			if e.reason != "" {
				if e.reason == "offloaded" {
					return nil, false, contracts.ErrUnsupported
				}
				return nil, false, contracts.ErrNotFound
			}
			return nil, true, nil
		}
		if e.offset != offset {
			return nil, false, contracts.ErrCorrupt
		}
		return e.bytes, false, nil
	}
}
func (c *Client) followBlobs(ctx context.Context, conn *websocket.Conn, events <-chan blobEvent, trigger <-chan struct{}) {
	for {
		select {
		case <-ctx.Done():
			return
		case <-trigger:
			refs, err := c.Store.BlobReferenceSet(ctx)
			if err != nil {
				continue
			}
			seen := map[string]bool{}
			for _, list := range refs {
				for _, ref := range list {
					if seen[ref.Digest] {
						continue
					}
					seen[ref.Digest] = true
					c.Blobs.Transfer(ctx, ref.Digest, &feedSource{conn: conn, events: events})
				}
			}
		}
	}
}
