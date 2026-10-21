package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"net/http"
	"os/exec"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	bridgeweb "github.com/JohanSardar/apple-bridge/internal/web"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func runWebUpload(ctx context.Context, s Scenario, level string) (e Evidence) {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e = Evidence{RunID: contracts.NewID("upload"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Metrics: map[string]any{}, Latency: map[string]Latency{}, Hosts: []string{"air"}}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	defer func() {
		e.Passed = len(e.Assertions) > 0
		for _, a := range e.Assertions {
			e.Passed = e.Passed && a.Passed
		}
	}()
	w, err := world.New(ctx, "sim", 0, 1, world.BlobOptions{Scope: "admin"})
	add("gateway_upload_world", err == nil)
	if err != nil {
		return
	}
	defer w.Close()
	endpoint := w.Gateway
	authority, err := trust.Open(endpoint.Home)
	if err != nil {
		add("serving_authority", false)
		return
	}
	token, _, err := authority.CreateDeviceSession("trusted")
	add("serving_device_session", err == nil)
	if err != nil {
		return
	}
	c := &webClient{ctx: ctx, origin: strings.Replace(endpoint.URL, "127.0.0.1", "localhost", 1), cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: token}}
	data := bytes.Repeat([]byte("Bridge Test upload bytes\n"), 100000)
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	switch s.ID {
	case "WEB-UPLOAD-RESUME":
		status, out := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(len(data)), "Upload-Digest": digest})
		add("resumable_upload_started", status == 200 && out.ID != "" && out.State == "uploading" && out.Offset == 0)
		if out.ID == "" {
			return
		}
		otherToken, _, otherErr := authority.CreateDeviceSession("trusted")
		other := &webClient{ctx: ctx, origin: c.origin, cookie: &http.Cookie{Name: bridgeweb.LocalSessionCookie, Value: otherToken}}
		otherStatus, _ := uploadRequest(other, "GET", "upload/"+out.ID, nil, nil)
		add("other_device_cannot_read_upload", otherErr == nil && otherStatus == 404)
		otherStatus, _ = uploadRequest(other, "DELETE", "upload/"+out.ID, nil, nil)
		add("other_device_cannot_abandon_upload", otherStatus == 404)
		add("ten_minute_upload_lease", time.Until(out.Expires) > 9*time.Minute && time.Until(out.Expires) <= 10*time.Minute)
		first := data[:1<<20]
		part, accepted := interruptUpload(c, out.ID, data)
		add("first_chunk_retained_after_request_interruption", accepted && part.State == "uploading")
		writtenExpiry := part.Expires
		status, part = uploadRequest(c, "GET", "upload/"+out.ID, nil, nil)
		add("status_reads_do_not_extend_lease", status == 200 && part.Expires.Equal(writtenExpiry))
		e.Metrics["interrupted_offset_bytes"] = part.Offset
		add("interruption_offset_discovered", status == 200 && part.Offset == int64(len(first)))
		status, part = uploadRequest(c, "POST", "upload", bytes.NewReader(data[len(first):]), map[string]string{"Upload-ID": out.ID, "Upload-Offset": fmt.Sprint(len(first))})
		e.Metrics["upload_bytes"] = part.Size
		add("resume_same_id_verified_ready", status == 200 && part.ID == out.ID && part.State == "ready" && part.Offset == int64(len(data)))
		status, _ = uploadRequest(c, "DELETE", "upload/"+out.ID, nil, nil)
		add("abandoned_upload_released", status == 200)
		status, _ = uploadRequest(c, "GET", "upload/"+out.ID, nil, nil)
		add("released_upload_absent", status == 404)
	case "WEB-UPLOAD-OVERSIZE":
		status, _ := uploadRequest(c, "POST", "upload", nil, map[string]string{"Upload-Length": fmt.Sprint(100*1024*1024 + 1)})
		add("oversize_declared_before_staging_rejected", status == 413)
		status, _ = uploadRequest(c, "POST", "upload", io.LimitReader(webZeroReader{}, 100*1024*1024+1), nil)
		add("oversize_stream_rejected", status == 413)
	case "WEB-UPLOAD-FEED":
		uploadFeedProof(ctx, w, &e, add)
	case "WEB-UPLOAD-MISSING-PART":
		uploadMissingPart(w, c, add)
	case "WEB-UPLOAD-LEASE":
		uploadApprovalLease(ctx, w, c, add)
	case "WEB-UPLOAD-LIMITS":
		uploadFileCountCheck(ctx, c, add)
	case "WEB-UPLOAD-DIGEST":
		status, _ := uploadRequest(c, "POST", "upload", bytes.NewReader(data), map[string]string{"Upload-Digest": strings.Repeat("0", 64)})
		add("mismatched_digest_rejected", status == 422)
	default:
		add("supported_upload_scenario", false)
	}
	return
}
