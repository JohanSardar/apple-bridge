package applepeer

import (
	"context"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"time"
)

func (w *Wire) deliverBlob(f contracts.IPCFrame) {
	w.blobMu.Lock()
	ch := w.blobPending[f.Handle]
	w.blobMu.Unlock()
	if ch != nil {
		select {
		case ch <- f:
		default:
		}
	}
}
func (w *Wire) ReadBlob(ctx context.Context, handle string, offset, length int64) ([]byte, bool, error) {
	w.blobMu.Lock()
	if w.blobPending == nil {
		w.blobPending = map[string]chan contracts.IPCFrame{}
	}
	if w.blobPending[handle] != nil {
		w.blobMu.Unlock()
		return nil, false, contracts.ErrConflict
	}
	ch := make(chan contracts.IPCFrame, 1)
	w.blobPending[handle] = ch
	w.blobMu.Unlock()
	defer func() { w.blobMu.Lock(); delete(w.blobPending, handle); w.blobMu.Unlock() }()
	if _, err := w.Send(contracts.IPCFrame{T: "BlobRead", Handle: handle, Offset: offset, Length: min(length, 128<<10)}); err != nil {
		return nil, false, err
	}
	timer := time.NewTimer(10 * time.Second)
	defer timer.Stop()
	select {
	case <-ctx.Done():
		return nil, false, ctx.Err()
	case <-timer.C:
		return nil, false, contracts.ErrUnavailable
	case f := <-ch:
		if f.T == "BlobUnavailable" {
			if f.Reason == "offloaded" {
				return nil, false, contracts.ErrUnsupported
			}
			return nil, false, contracts.ErrNotFound
		}
		if f.Offset != offset || len(f.Bytes) > 128<<10 {
			return nil, false, contracts.ErrCorrupt
		}
		return f.Bytes, f.EOF, nil
	}
}
