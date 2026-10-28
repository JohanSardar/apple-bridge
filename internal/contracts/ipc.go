package contracts

import (
	"encoding/binary"
	"encoding/json"
	"io"
	"time"
)

const MaxFrame = 256 * 1024
const MaxQueryFrame = MaxQueryJSON + (64 << 10)
const ByteCredit = 4 * 1024 * 1024
const IPCProtocol = "ipc/1"
const FeedProtocol = "feed/1"

type Observation struct {
	Kind        string          `json:"kind"`
	Collection  string          `json:"collection"`
	NativeKey   string          `json:"native_key"`
	ScopedAlias string          `json:"scoped_alias,omitempty"`
	Data        json.RawMessage `json:"data,omitempty"`
	ObservedAt  time.Time       `json:"observed_at"`
	Revalidated bool            `json:"revalidated,omitempty"`
}

type Capture struct {
	Live        bool              `json:"live,omitempty"`
	BlobHandles map[string]string `json:"blob_handles,omitempty"`

	Source             Source        `json:"source"`
	Generation         string        `json:"generation"`
	Predecessor        string        `json:"predecessor"`
	Checkpoint         string        `json:"checkpoint"`
	Observations       []Observation `json:"observations"`
	ScanID             string        `json:"scan_id,omitempty"`
	Scope              []string      `json:"scope,omitempty"`
	ScanStartedAt      time.Time     `json:"scan_started_at,omitempty"`
	CompletionEvidence string        `json:"completion_evidence,omitempty"`
}

type Checkpoint struct {
	RefreshBlobs bool   `json:"refresh_blobs,omitempty"`
	Generation   string `json:"generation"`
	Checkpoint   string `json:"checkpoint"`
}

type IPCFrame struct {
	ID            string          `json:"id,omitempty"`
	Op            string          `json:"op,omitempty"`
	Args          json.RawMessage `json:"args,omitempty"`
	DeadlineMS    int             `json:"deadline_ms,omitempty"`
	ResultJSON    json.RawMessage `json:"result_json,omitempty"`
	Blobs         []QueryBlob     `json:"blobs,omitempty"`
	Code          string          `json:"code,omitempty"`
	Origin        string          `json:"origin,omitempty"`
	Handle        string          `json:"handle,omitempty"`
	Offset        int64           `json:"offset,omitempty"`
	Length        int64           `json:"length,omitempty"`
	Bytes         []byte          `json:"bytes,omitempty"`
	Digest        string          `json:"digest,omitempty"`
	EOF           bool            `json:"eof,omitempty"`
	CommandID     string          `json:"command_id,omitempty"`
	Principal     string          `json:"principal,omitempty"`
	RequestHash   string          `json:"request_hash,omitempty"`
	Action        *NativeAction   `json:"action,omitempty"`
	ExpectedState json.RawMessage `json:"expected_state,omitempty"`
	ExpiresAt     time.Time       `json:"expires_at,omitempty"`
	Ticket        string          `json:"ticket,omitempty"`
	NativeState   json.RawMessage `json:"native_state,omitempty"`
	Route         string          `json:"route,omitempty"`
	Fingerprint   string          `json:"fingerprint,omitempty"`
	CandidateIDs  []string        `json:"candidate_ids,omitempty"`
	Disposition   Disposition     `json:"disposition,omitempty"`
	Evidence      json.RawMessage `json:"evidence,omitempty"`
	Error         *Error          `json:"error,omitempty"`

	T            string                `json:"t"`
	Protocol     string                `json:"protocol,omitempty"`
	OwnerFence   uint64                `json:"owner_fence,omitempty"`
	Checkpoints  map[Source]Checkpoint `json:"checkpoints,omitempty"`
	ByteCredit   int                   `json:"byte_credit,omitempty"`
	Credit       int                   `json:"credit,omitempty"`
	Capture      *Capture              `json:"capture,omitempty"`
	Source       Source                `json:"source,omitempty"`
	Checkpoint   string                `json:"checkpoint,omitempty"`
	Capabilities map[Source][]string   `json:"capabilities,omitempty"`
	Coverage     map[Source]Coverage   `json:"coverage,omitempty"`
	HelperBuild  string                `json:"helper_build,omitempty"`
	OSBuild      string                `json:"os_build,omitempty"`
	State        string                `json:"state,omitempty"`
	Reason       string                `json:"reason,omitempty"`
	Nonce        string                `json:"nonce,omitempty"`
	PeerNonce    string                `json:"peer_nonce,omitempty"`
	Proof        string                `json:"proof,omitempty"`
}

type Open IPCFrame
type Ready IPCFrame
type Committed IPCFrame
type SourceState IPCFrame
type Ping IPCFrame
type Pong IPCFrame
type Close IPCFrame

type Prepare struct {
	Origin        string          `json:"origin,omitempty"`
	T             string          `json:"t"`
	CommandID     string          `json:"command_id"`
	Principal     string          `json:"principal"`
	RequestHash   string          `json:"request_hash"`
	Action        json.RawMessage `json:"action"`
	ExpectedState json.RawMessage `json:"expected_state"`
	ExpiresAt     time.Time       `json:"expires_at"`
}
type Prepared struct {
	T            string          `json:"t"`
	CommandID    string          `json:"command_id"`
	Ticket       string          `json:"ticket"`
	NativeState  json.RawMessage `json:"native_state"`
	Route        string          `json:"route"`
	Fingerprint  string          `json:"fingerprint"`
	CandidateIDs []string        `json:"candidate_ids"`
}
type Dispatch struct {
	Origin     string `json:"origin,omitempty"`
	CommandID  string `json:"command_id"`
	T          string `json:"t"`
	Ticket     string `json:"ticket"`
	OwnerFence uint64 `json:"owner_fence"`
}
type Outcome struct {
	T           string          `json:"t"`
	CommandID   string          `json:"command_id"`
	Disposition Disposition     `json:"disposition"`
	Evidence    json.RawMessage `json:"evidence"`
}
type Resolve struct {
	T         string `json:"t"`
	CommandID string `json:"command_id"`
}
type BlobUnavailable struct {
	T      string `json:"t"`
	Handle string `json:"handle,omitempty"`
	Digest string `json:"digest,omitempty"`
	Reason string `json:"reason"`
}
type BlobRead struct {
	T      string `json:"t"`
	Handle string `json:"handle"`
	Offset int64  `json:"offset"`
	Length int64  `json:"length"`
}
type BlobChunk struct {
	T      string `json:"t"`
	Handle string `json:"handle"`
	Offset int64  `json:"offset"`
	Bytes  []byte `json:"bytes"`
	Digest string `json:"digest"`
	EOF    bool   `json:"eof"`
}

func EncodeFrame(v any) ([]byte, error) {
	b, err := MarshalQueryJSON(v)
	if err != nil {
		return nil, err
	}
	var head struct {
		T string `json:"t"`
	}
	valid := json.Unmarshal(b, &head) == nil && head.T != ""

	if len(b) > MaxQueryFrame {
		return nil, ErrCapacity
	}
	if !valid {
		return nil, ErrInvalid
	}
	if len(b) > MaxFrame && head.T != "QueryResult" {
		return nil, ErrCapacity
	}
	out := make([]byte, 4+len(b))
	binary.BigEndian.PutUint32(out, uint32(len(b)))
	copy(out[4:], b)
	return out, nil
}

func WriteFrame(w io.Writer, v any) (int, error) {
	b, err := EncodeFrame(v)
	if err != nil {
		return 0, err
	}
	written := 0
	for written < len(b) {
		n, err := w.Write(b[written:])
		written += n
		if err != nil {
			return written, err
		}
		if n == 0 {
			return written, io.ErrShortWrite
		}
	}
	return written, nil
}

func ReadFrame(r io.Reader, v any) (int, error) {
	var head [4]byte
	if _, err := io.ReadFull(r, head[:]); err != nil {
		return 0, err
	}
	n := binary.BigEndian.Uint32(head[:])
	if n == 0 || n > MaxQueryFrame {
		return 4, ErrCapacity
	}
	b := make([]byte, n)
	if _, err := io.ReadFull(r, b); err != nil {
		return 4, err
	}
	var kind struct {
		T string `json:"t"`
	}
	if json.Unmarshal(b, &kind) != nil || kind.T == "" {
		return int(n) + 4, ErrInvalid
	}
	if n > MaxFrame && kind.T != "QueryResult" {
		return int(n) + 4, ErrCapacity
	}
	return int(n) + 4, json.Unmarshal(b, v)
}
