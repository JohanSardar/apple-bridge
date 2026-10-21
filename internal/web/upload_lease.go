package web

import (
	"errors"
	"io"
	"net/http"
	"strconv"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *server) uploadLease(w http.ResponseWriter, r *http.Request) {
	if s.replicaUpload() {
		reject(w, 503, "unavailable")
		return
	}
	p := current(r).Principal
	if s.deps.Authority.Authorize(p, "messages:write") != nil {
		reject(w, 401, "unauthorized")
		return
	}
	if s.deps.Blobs == nil {
		reject(w, 503, "unavailable")
		return
	}
	action := "status"
	if r.Method == "DELETE" {
		action = "abandon"
	}
	out, err := s.deps.Blobs.Upload(r.Context(), p.ID, contracts.UploadRequest{Action: action, ID: r.PathValue("id")})
	if err != nil {
		uploadError(w, err)
		return
	}
	respond(w, 200, out)
}
func (s *server) streamUpload(w http.ResponseWriter, r *http.Request) {
	p := current(r).Principal
	size := r.ContentLength
	if size <= 0 {
		size = -1
	}
	declared := r.Header.Get("Upload-Length")
	var err error
	if declared != "" {
		size, err = strconv.ParseInt(declared, 10, 64)
	}
	if err != nil || size < -1 || declared != "" && size < 0 {
		reject(w, 400, "invalid")
		return
	}
	if size > contracts.MessageFileLimit {
		uploadError(w, contracts.ErrTooLarge)
		return
	}
	id := r.Header.Get("Upload-ID")
	action := "status"
	if id == "" {
		id = contracts.NewID("upload")
		action = "begin"
	}
	out, err := s.deps.Blobs.Upload(r.Context(), p.ID, contracts.UploadRequest{Action: action, ID: id, Size: size, Digest: r.Header.Get("Upload-Digest")})
	if err != nil {
		uploadError(w, err)
		return
	}
	if action == "begin" && declared != "" && r.ContentLength == 0 {
		respond(w, 200, out)
		return
	}
	offset := int64(0)
	if raw := r.Header.Get("Upload-Offset"); raw != "" {
		offset, err = strconv.ParseInt(raw, 10, 64)
	}
	if err != nil || offset != out.Offset {
		uploadError(w, contracts.ErrConflict)
		return
	}
	body := http.MaxBytesReader(w, r.Body, contracts.MessageFileLimit)
	for {
		chunk, readErr := io.ReadAll(io.LimitReader(body, contracts.BlobChunkSize))
		n := len(chunk)
		if n > 0 {
			out, err = s.deps.Blobs.AppendUpload(r.Context(), p.ID, id, out.Offset, chunk)
		}
		if err != nil {
			uploadError(w, err)
			return
		}
		if readErr != nil {
			uploadError(w, readErr)
			return
		}
		if n < contracts.BlobChunkSize {
			break
		}
	}
	if out.Size < 0 || out.Size == out.Offset {
		out, err = s.deps.Blobs.Upload(r.Context(), p.ID, contracts.UploadRequest{Action: "commit", ID: id, Digest: r.Header.Get("Upload-Digest")})
	}
	if err != nil {
		uploadError(w, err)
		return
	}
	respond(w, 200, out)
}
func uploadError(w http.ResponseWriter, err error) {
	var large *http.MaxBytesError
	switch {
	case errors.As(err, &large), errors.Is(err, contracts.ErrTooLarge):
		reject(w, 413, "too_large")
	case errors.Is(err, contracts.ErrCorrupt):
		reject(w, 422, "corrupt")
	case errors.Is(err, contracts.ErrNotFound):
		reject(w, 404, "not_found")
	case errors.Is(err, contracts.ErrConflict):
		reject(w, 409, "conflict")
	case errors.Is(err, contracts.ErrInvalid):
		reject(w, 400, "invalid")
	case errors.Is(err, contracts.ErrCapacity):
		reject(w, 507, "capacity")
	default:
		reject(w, 503, safeError(err).Code)
	}
}

func (s *server) replicaUpload() bool {
	return s.deps.Operations != nil && s.deps.Operations.LocalStatus != nil && s.deps.Operations.LocalStatus().Role == "replica"
}
