package contracts

import (
	"encoding/json"
	"time"
)

type Batch struct {
	Epoch                string   `json:"epoch"`
	SubscriptionRevision uint64   `json:"subscription_revision"`
	After                uint64   `json:"after"`
	Through              uint64   `json:"through"`
	Changes              []Change `json:"changes"`
}

type CommandResponse struct {
	Result     json.RawMessage `json:"result,omitempty"`
	Error      *Error          `json:"error,omitempty"`
	LookupOnly bool            `json:"lookup_only,omitempty"`
}

type FeedFrame struct {
	ClockProtocol string           `json:"clock_protocol,omitempty"`
	Clock         *ClockSample     `json:"clock,omitempty"`
	Request       json.RawMessage  `json:"request,omitempty"`
	Delegation    string           `json:"delegation,omitempty"`
	Response      *CommandResponse `json:"response,omitempty"`
	Offset        int64            `json:"offset,omitempty"`

	T                    string    `json:"t"`
	Protocol             string    `json:"protocol,omitempty"`
	ReplicaID            string    `json:"replica_id,omitempty"`
	GatewayID            string    `json:"gateway_id,omitempty"`
	Cursor               *Cursor   `json:"cursor,omitempty"`
	SchemaOffer          []int     `json:"schema_offer,omitempty"`
	Schema               int       `json:"schema,omitempty"`
	Credits              int       `json:"credits,omitempty"`
	Epoch                string    `json:"epoch,omitempty"`
	SubscriptionRevision uint64    `json:"subscription_revision,omitempty"`
	Scope                []Grant   `json:"scope,omitempty"`
	Head                 uint64    `json:"head,omitempty"`
	RetentionFloor       uint64    `json:"retention_floor,omitempty"`
	ID                   string    `json:"id,omitempty"`
	S                    uint64    `json:"s,omitempty"`
	Count                int       `json:"count,omitempty"`
	Digest               string    `json:"digest,omitempty"`
	ExpiresAt            time.Time `json:"expires_at,omitempty"`
	Ordinal              int       `json:"ordinal,omitempty"`
	Records              []Record  `json:"records,omitempty"`
	Batch                *Batch    `json:"batch,omitempty"`
	Reason               string    `json:"reason,omitempty"`
	At                   time.Time `json:"at,omitempty"`
}

type Hello FeedFrame
type Welcome FeedFrame
type SnapshotBegin FeedFrame
type SnapshotPage FeedFrame
type SnapshotEnd FeedFrame
type Ack FeedFrame
type Reset FeedFrame
type Heartbeat FeedFrame

type BlobWant FeedFrame
type BlobDone FeedFrame
