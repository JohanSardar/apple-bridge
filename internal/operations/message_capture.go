package operations

import (
	"context"
	"encoding/json"
	"errors"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (g *LocalGateway) messageCapturePending(r contracts.CommandRecord, f contracts.IPCFrame) bool {
	if f.Disposition != contracts.DispositionLocallyObserved && f.Disposition != contracts.DispositionDelivered {
		return false
	}
	var evidence struct {
		NativeKey      string `json:"native_key"`
		Fingerprint    string `json:"fingerprint"`
		Observed       bool   `json:"observed"`
		Checkpoint     string `json:"capture_checkpoint"`
		Ambiguous      bool   `json:"ambiguous"`
		CandidateCount int    `json:"candidate_count"`
	}
	if json.Unmarshal(f.Evidence, &evidence) != nil || evidence.Ambiguous || evidence.CandidateCount > 1 || !evidence.Observed || evidence.NativeKey == "" || evidence.Fingerprint != r.RequestHash {
		return false
	}
	_, err := g.Peer.CapturedSequence(contracts.Messages, evidence.Checkpoint, "messages.message", evidence.NativeKey)
	return errors.Is(err, contracts.ErrNotFound)
}

func (g *LocalGateway) awaitMessageCapture(ctx context.Context, r contracts.CommandRecord, f contracts.IPCFrame) contracts.IPCFrame {
	if r.Command.Op != "messages.send" || r.Source != contracts.Messages || r.State != contracts.CommandDispatched {
		return f
	}
	changed := g.Peer.CaptureChanges(contracts.Messages)
	if !g.messageCapturePending(r, f) {
		return f
	}
	bounded, stop := context.WithTimeout(ctx, 10*time.Second)
	defer stop()
	for g.messageCapturePending(r, f) {
		select {
		case <-bounded.Done():
			return f
		case <-changed:
		}
		changed = g.Peer.CaptureChanges(contracts.Messages)
		if bounded.Err() != nil {
			return f
		}
		next, err := g.Peer.Exchange(bounded, contracts.IPCFrame{T: "Resolve", CommandID: r.Command.ID})
		if err != nil {
			return f
		}
		f = next
	}
	return f
}
