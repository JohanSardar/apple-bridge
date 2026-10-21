package replication

import (
	"context"
	"errors"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"github.com/coder/websocket"
	"io"
	"sort"
	"time"
)

func (s *Server) serveBlobs(ctx context.Context, conn *websocket.Conn, p trust.Principal, wants <-chan contracts.FeedFrame) {
	waiting := map[string]bool{}
	var changed <-chan struct{}
	if s.Blobs != nil {
		changed = s.Blobs.Changed()
	}
	for {
		select {
		case <-ctx.Done():
			return
		case <-changed:
			changed = s.Blobs.Changed()
			for d := range waiting {
				state := s.Blobs.Progress().Entries[d].State
				if state != "present" && state != "failed" {
					continue
				}
				if _, err := s.Store.BlobGrant(ctx, p.Visibility(), d); err == nil {
					send(ctx, conn, contracts.FeedFrame{T: "BlobAvailable", Digest: d})
				}
				delete(waiting, d)
			}
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
				switch s.Blobs.Progress().Entries[want.Digest].State {
				case "offloaded":
					reason = "offloaded"
				case "failed":
					reason = "failed"
				case "present", "pending", "disk_full", "over_cap":
					reason = "pending"
					waiting[want.Digest] = true
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
				if e.reason == "pending" || e.reason == "unavailable" {
					return nil, false, contracts.ErrUnavailable
				}
				if e.reason == "failed" {
					return nil, false, contracts.ErrInvalid
				}
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
			list := []contracts.BlobReference{}
			for _, refs := range refs {
				list = append(list, refs...)
			}
			sort.Slice(list, func(i, j int) bool {
				if list[i].Variant != list[j].Variant {
					return list[i].Variant
				}
				return list[i].Digest < list[j].Digest
			})
			seen := map[string]bool{}
			variantsPending := false
			for _, ref := range list {
				if !ref.Variant && variantsPending {
					continue
				}
				if ref.AwaitingVariants || seen[ref.Digest] {
					continue
				}
				seen[ref.Digest] = true
				source := &feedSource{conn: conn, events: events}
				err := c.Blobs.Transfer(ctx, ref.Digest, source)
				if ref.Variant && err != nil && !errors.Is(err, contracts.ErrCapacity) && !errors.Is(err, contracts.ErrNotFound) && !errors.Is(err, contracts.ErrUnsupported) && !errors.Is(err, contracts.ErrInvalid) && !errors.Is(err, contracts.ErrCorrupt) {
					variantsPending = true
				}
				if source.started {
					source.drain(ctx)
				}
			}
		}
	}
}

func (s *feedSource) drain(ctx context.Context) {
	for s.started {
		select {
		case <-ctx.Done():
			return
		case e := <-s.events:
			if e.done {
				s.started = false
			}
		}
	}
}
