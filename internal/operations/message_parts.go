package operations

import (
	"context"
	"encoding/json"
	"errors"
	"strconv"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type messagePartEvidence struct {
	Index          *int   `json:"index"`
	Kind           string `json:"kind"`
	NativeKey      string `json:"native_key"`
	Disposition    string `json:"disposition"`
	Observed       bool   `json:"observed"`
	Ambiguous      bool   `json:"ambiguous"`
	CandidateCount int    `json:"candidate_count"`
}

func (g *LocalGateway) messageParts(ctx context.Context, r *contracts.CommandRecord, f *contracts.IPCFrame) bool {
	if r.Command.Op != "messages.send" {
		return false
	}
	var proof struct {
		Fingerprint    string            `json:"fingerprint"`
		Checkpoint     string            `json:"capture_checkpoint"`
		Parts          []json.RawMessage `json:"parts"`
		Ambiguous      bool              `json:"ambiguous"`
		CandidateCount int               `json:"candidate_count"`
	}
	if json.Unmarshal(f.Evidence, &proof) != nil || proof.Fingerprint != r.RequestHash || len(proof.Parts) == 0 && len(r.Parts) == 0 {
		return false
	}
	args, err := contracts.ValidateMessagesCommand(r.Command)
	if err != nil {
		return false
	}
	next := []contracts.CommandPart{}
	if args.Text != "" {
		next = append(next, contracts.CommandPart{Index: 0, Kind: "text", Disposition: "indeterminate"})
	}
	for _, file := range args.Files {
		next = append(next, contracts.CommandPart{Index: len(next), Kind: "attachment", UploadID: file.UploadID, Disposition: "indeterminate"})
	}
	if len(r.Parts) > 0 {
		if len(r.Parts) != len(next) {
			return false
		}
		copy(next, r.Parts)
	}
	var previous struct {
		Parts []json.RawMessage `json:"parts"`
	}
	json.Unmarshal(r.Evidence, &previous)
	raw := make([]json.RawMessage, len(next))
	for _, value := range previous.Parts {
		var part messagePartEvidence
		if json.Unmarshal(value, &part) == nil && part.Index != nil && *part.Index >= 0 && *part.Index < len(raw) {
			raw[*part.Index] = value
		}
	}
	seen := map[int]bool{}
	problems := map[string]string{}
	for _, value := range proof.Parts {
		var evidence messagePartEvidence
		if json.Unmarshal(value, &evidence) != nil || evidence.Index == nil || *evidence.Index < 0 || *evidence.Index >= len(next) {
			return false
		}
		index := *evidence.Index
		if seen[index] || evidence.Kind != next[index].Kind {
			return false
		}
		seen[index] = true
		part := next[index]
		part.RecordID = ""
		part.Disposition = "indeterminate"
		captureBound := false
		switch evidence.Disposition {
		case "delivery_failed":
			part.Disposition = "delivery_failed"
		case "failed_no_effect", "not_dispatched", "rejected_before_dispatch":
			part.Disposition = "failed_no_effect"
		}
		if !proof.Ambiguous && proof.CandidateCount <= 1 && !evidence.Ambiguous && evidence.CandidateCount <= 1 && evidence.Observed && evidence.NativeKey != "" && (evidence.Disposition == "locally_observed" || evidence.Disposition == "delivered" || evidence.Disposition == "delivery_failed") {
			cursor, problem := g.Peer.CapturedSequence(contracts.Messages, proof.Checkpoint, "messages.message", evidence.NativeKey)
			if problem == nil && cursor.Sequence > 0 && cursor.Epoch == g.State.Head().Epoch {
				captureBound = true
				if evidence.Disposition == "locally_observed" {
					part.Disposition = "observed"
				} else {
					part.Disposition = evidence.Disposition
				}
				id, problem := g.State.ObservedRecordID(ctx, "messages.message", evidence.NativeKey, cursor)
				if problem == nil {
					part.RecordID = id
				} else if !errors.Is(problem, contracts.ErrNotFound) {
					code := contracts.ErrUnavailable.Code
					var typed *contracts.Error
					if errors.As(problem, &typed) {
						code = typed.Code
					}
					problems[strconv.Itoa(index)] = code
				}
				if cursor.Sequence > r.CommitSeq {
					r.CommitSeq = cursor.Sequence
				}
			}
		}
		prior := next[index]
		var priorProof messagePartEvidence
		json.Unmarshal(raw[index], &priorProof)
		boundFailure := prior.Disposition == "observed" && part.Disposition == "delivery_failed" && captureBound && priorProof.NativeKey != "" && evidence.NativeKey == priorProof.NativeKey && (prior.RecordID == "" || part.RecordID == prior.RecordID)
		if prior.Disposition != "indeterminate" && part.Disposition != prior.Disposition && !(prior.Disposition == "observed" && part.Disposition == "delivered") && !boundFailure {
			continue
		}
		if prior.Disposition != "indeterminate" && priorProof.NativeKey != "" && evidence.NativeKey != "" && priorProof.NativeKey != evidence.NativeKey {
			continue
		}
		if prior.RecordID != "" {
			if part.RecordID != "" && part.RecordID != prior.RecordID {
				continue
			}
			part.RecordID = prior.RecordID
		}
		if prior == part && raw[index] != nil {
			continue
		}
		next[index] = part
		raw[index] = value
	}
	var details map[string]json.RawMessage
	json.Unmarshal(f.Evidence, &details)
	merged := []json.RawMessage{}
	for _, part := range raw {
		if part != nil {
			merged = append(merged, part)
		}
	}
	details["parts"], _ = json.Marshal(merged)
	if len(problems) > 0 {
		details["part_record_id_resolution_error"], _ = json.Marshal(problems)
	}
	f.Evidence, _ = json.Marshal(details)
	r.Parts = next
	return true
}

func aggregateMessageParts(parts []contracts.CommandPart) (contracts.CommandState, contracts.Disposition) {
	observed, delivered := true, true
	for _, part := range parts {
		if part.Disposition == "delivery_failed" || part.Disposition == "failed_no_effect" {
			return contracts.CommandDeliveryFailed, contracts.DispositionDeliveryFailed
		}
		observed = observed && (part.Disposition == "observed" || part.Disposition == "delivered")
		delivered = delivered && part.Disposition == "delivered"
	}
	if delivered {
		return contracts.CommandDelivered, contracts.DispositionDelivered
	}
	if observed {
		return contracts.CommandObserved, contracts.DispositionLocallyObserved
	}
	return contracts.CommandIndeterminate, contracts.DispositionIndeterminate
}

func sameMessageParts(a, b []contracts.CommandPart) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

func resolvableMessageParts(r contracts.CommandRecord) bool {
	if r.Source != contracts.Messages {
		return false
	}
	if r.State == contracts.CommandObserved {
		return true
	}
	if r.State != contracts.CommandDeliveryFailed {
		return false
	}
	for _, part := range r.Parts {
		if part.Disposition == "indeterminate" || part.Disposition == "observed" || part.Disposition == "delivery_failed" && part.RecordID == "" {
			return true
		}
	}
	return false
}
