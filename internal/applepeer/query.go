package applepeer

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"io"
)

type queryBlobReader struct {
	ctx     context.Context
	wire    *Wire
	blob    contracts.QueryBlob
	offset  int64
	pending []byte
}

func (r *queryBlobReader) Read(p []byte) (int, error) {
	if len(p) == 0 {
		return 0, nil
	}
	if len(r.pending) == 0 {
		if r.offset == r.blob.Size {
			return 0, io.EOF
		}
		data, eof, err := r.wire.ReadBlob(r.ctx, r.blob.Handle, r.offset, min(128<<10, r.blob.Size-r.offset))
		if err != nil {
			return 0, err
		}
		if len(data) == 0 || r.offset+int64(len(data)) > r.blob.Size || eof && r.offset+int64(len(data)) != r.blob.Size {
			return 0, contracts.ErrCorrupt
		}
		r.pending = data
		r.offset += int64(len(data))
	}
	n := copy(p, r.pending)
	r.pending = r.pending[n:]
	return n, nil
}
func (c *Commands) QueryBlob(ctx context.Context, b contracts.QueryBlob) (io.Reader, error) {
	c.mu.Lock()
	s := c.session
	c.mu.Unlock()
	if s == nil {
		return nil, contracts.ErrUnavailable
	}
	return &queryBlobReader{ctx: ctx, wire: s.wire, blob: b}, nil
}
