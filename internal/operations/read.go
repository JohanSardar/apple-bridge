package operations

import (
	"context"
	"encoding/json"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/state"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type Service struct {
	Authority   *trust.Authority
	Target      CommandTarget
	State       *state.Store
	LocalStatus func() contracts.ReplicaStatus
}

func (s *Service) Read(ctx context.Context, p trust.Principal, name string, q contracts.ReadQuery) (output contracts.ReadResult, resultErr error) {
	defer func() {
		if err := s.recordAgentRead(context.WithoutCancel(ctx), p, name); err != nil && resultErr == nil {
			resultErr = err
		}
	}()
	if err := contracts.ValidateQuery(q); err != nil {
		return contracts.ReadResult{}, err
	}
	if name == "replica.status" || q.Collection == "system.replica" {
		result := contracts.ReadResult{Records: []contracts.Record{}, Cursor: s.State.Head()}
		if !p.Visibility()[contracts.System] {
			return result, nil
		}
		if s.LocalStatus != nil {
			status, err := s.LocalReplicaStatus(ctx, p)
			if err != nil {
				return result, err
			}
			result.Replica = &status
			data, err := json.Marshal(status)
			if err != nil {
				return result, err
			}
			result.Records = append(result.Records, contracts.Record{Collection: "system.replica", ID: "local", Version: 1, Source: contracts.System, ObservedAt: time.Now().UTC(), Data: data})
		}
		return result, nil
	}
	switch name {
	case "records.get":
		if q.Collection == "" || q.ID == "" {
			return contracts.ReadResult{}, contracts.ErrInvalid
		}
	case "records.query":
	case "search":
		if q.Text == "" {
			return contracts.ReadResult{}, contracts.ErrInvalid
		}
	case "sources.status":
		q = contracts.ReadQuery{Collection: "system.source"}
	default:
		return contracts.ReadResult{}, contracts.ErrUnsupported
	}
	var result contracts.ReadResult
	var err error
	if name == "sources.status" {
		result, err = s.State.SourceStatuses(ctx, p.Visibility())
	} else {
		result, err = s.State.Read(ctx, p.Visibility(), q)
	}
	if err == nil && (name == "sources.status" || name == "replica.status") && s.LocalStatus != nil {
		var status contracts.ReplicaStatus
		status, err = s.LocalReplicaStatus(ctx, p)
		result.Replica = &status
	}
	return result, err
}
func (s *Service) Head() contracts.Cursor { return s.State.Head() }
func (s *Service) Observe(ctx context.Context, p trust.Principal, after contracts.Cursor) (*state.ChangeIterator, error) {
	return s.State.Changes(ctx, p.Visibility(), after)
}

func (s *Service) BlobGrant(ctx context.Context, p trust.Principal, id string) (output trust.RecordGrant, resultErr error) {
	defer func() {
		if err := s.recordAgentRead(context.WithoutCancel(ctx), p, "blobs.get"); err != nil && resultErr == nil {
			resultErr = err
		}
	}()
	return s.State.RecordGrant(ctx, p.Visibility(), id)
}

func (s *Service) recordAgentRead(ctx context.Context, p trust.Principal, name string) error {
	if p.Kind != "agent" {
		return nil
	}
	at := time.Now().UTC().Truncate(time.Minute)
	return s.State.RecordAgentActivity(ctx, state.AgentActivity{ID: p.ID + ":" + name + ":" + at.Format(time.RFC3339), Principal: p.ID, Operation: name, Kind: "read", State: "read", At: at})
}
