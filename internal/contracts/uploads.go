package contracts

import "time"

const MessageFileLimit int64 = 100 << 20
const MessageFileCount = 10
const UploadLease = 10 * time.Minute

type UploadRequest struct {
	ChunkID string `json:"chunk_id,omitempty"`
	Action  string `json:"action"`
	ID      string `json:"upload_id"`
	Offset  int64  `json:"offset,omitempty"`
	Size    int64  `json:"size"`
	Digest  string `json:"digest,omitempty"`
}

type UploadStatus struct {
	ID        string    `json:"upload_id"`
	Offset    int64     `json:"offset"`
	Size      int64     `json:"size"`
	State     string    `json:"state"`
	ExpiresAt time.Time `json:"expires_at"`
	Digest    string    `json:"digest,omitempty"`
}

var ErrTooLarge = &Error{Code: "too_large", Message: "file exceeds upload limit"}
