package contracts

import (
	"encoding/json"
	"strings"
)

type CaptureArgs struct {
	Blob    string  `json:"blob,omitempty"`
	BlobID  string  `json:"blob_id,omitempty"`
	Display *int    `json:"display,omitempty"`
	Window  *int    `json:"window,omitempty"`
	Seconds float64 `json:"seconds,omitempty"`
	File    string  `json:"file,omitempty"`
	Printer string  `json:"printer,omitempty"`
}

func CaptureOperation(op string) bool {
	switch op {
	case "capture.screenshot", "capture.camera_photo", "capture.record_audio", "mac.screen_record", "print.document":
		return true
	}
	return false
}
func ValidateCapture(c Command) error {
	var a CaptureArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return ErrInvalid
	}
	if c.Op != "print.document" && (a.Blob != "" || a.BlobID != "") {
		return ErrInvalid
	}
	switch c.Op {
	case "capture.screenshot":
		if a.Display != nil && a.Window != nil || a.Display != nil && *a.Display <= 0 || a.Window != nil && *a.Window <= 0 || a.Seconds != 0 || a.File != "" || a.Printer != "" {
			return ErrInvalid
		}
	case "capture.camera_photo":
		if a != (CaptureArgs{}) {
			return ErrInvalid
		}
	case "capture.record_audio", "mac.screen_record":
		limit := 60.0
		if c.Op == "mac.screen_record" {
			limit = 30
		}
		if a.Seconds < 1 || a.Seconds > limit || a.Display != nil || a.Window != nil || a.File != "" || a.Printer != "" {
			return ErrInvalid
		}
	case "print.document":
		if (strings.TrimSpace(a.File) == "" && (!ValidDigest(a.Blob) || !strings.HasPrefix(a.BlobID, "qry_"))) || (a.File != "" && (a.Blob != "" || a.BlobID != "")) || strings.TrimSpace(a.Printer) == "" || a.Display != nil || a.Window != nil || a.Seconds != 0 {
			return ErrInvalid
		}
	default:
		return ErrUnsupported
	}
	return nil
}
func CaptureEvidence(raw json.RawMessage, hash string) bool {
	var e struct {
		Digest   string `json:"digest"`
		Size     int64  `json:"size"`
		Observed bool   `json:"observed"`
		Prepared struct {
			Hash string `json:"request_hash"`
		} `json:"prepared"`
	}
	return json.Unmarshal(raw, &e) == nil && ValidDigest(e.Digest) && e.Size > 0 && e.Size <= 128<<20 && e.Observed && hash != "" && e.Prepared.Hash == hash
}
func init() {
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "capture.screenshot", Version: 1, Kind: CommandOp, Source: "capture", Scope: "capture:write", Capability: "capture.screenshot", Input: Command{}, Output: CommandRecord{}, Description: "Capture an artifact delivered through the blob store"},
			{Name: "capture.camera_photo", Version: 1, Kind: CommandOp, Source: "capture", Scope: "capture:write", Capability: "capture.camera_photo", Input: Command{}, Output: CommandRecord{}, Description: "Capture an artifact delivered through the blob store"},
			{Name: "capture.record_audio", Version: 1, Kind: CommandOp, Source: "capture", Scope: "capture:write", Capability: "capture.record_audio", Input: Command{}, Output: CommandRecord{}, Description: "Capture an artifact delivered through the blob store"},
		}
	})
}
