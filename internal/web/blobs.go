package web

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (s *server) upload(w http.ResponseWriter, r *http.Request) {
	p := current(r).Principal
	if s.deps.Authority.Authorize(p, "messages:write") != nil {
		reject(w, 401, "unauthorized")
		return
	}
	if s.deps.Blobs == nil {
		reject(w, 503, "unavailable")
		return
	}
	if r.Header.Get("Upload-ID") != "" || r.Header.Get("Upload-Length") != "" || r.Header.Get("Upload-Digest") != "" || s.deps.Operations != nil && s.deps.Operations.UploadForwarding() {
		s.streamUpload(w, r)
		return
	}
	id, err := s.deps.Blobs.StageMessageUpload(r.Context(), p.ID, http.MaxBytesReader(w, r.Body, 100*1024*1024))
	if err != nil {
		var large *http.MaxBytesError
		if errors.As(err, &large) {
			reject(w, 413, "too_large")
		} else if errors.Is(err, contracts.ErrCapacity) {
			reject(w, 507, "capacity")
		} else {
			reject(w, 503, safeError(err).Code)
		}
		return
	}
	out, err := s.deps.Blobs.Upload(r.Context(), p.ID, contracts.UploadRequest{Action: "status", ID: id})
	if err != nil {
		uploadError(w, err)
		return
	}
	if s.deps.Operations != nil {
		out, err = s.deps.Operations.ForwardUpload(r.Context(), p, out)
	}
	if err != nil {
		uploadError(w, err)
		return
	}
	respond(w, 200, out)
}
func (s *server) blob(w http.ResponseWriter, r *http.Request) {
	p := current(r).Principal
	digest := r.PathValue("digest")
	if !contracts.ValidDigest(digest) {
		reject(w, 400, "invalid")
		return
	}
	grant, err := s.deps.Operations.BlobGrant(r.Context(), p, r.URL.Query().Get("record"))
	if err != nil || !grant.AllowsBlob(digest) {
		reject(w, 401, "unauthorized")
		return
	}
	variant := r.URL.Query().Get("variant")
	if variant == "" {
		variant = "original"
	}
	if variant != "original" && variant != "thumb" && variant != "display" && variant != "video" && variant != "poster" && variant != "audio" && variant != "live" {
		reject(w, 400, "invalid")
		return
	}
	result, err := s.deps.Operations.Read(r.Context(), p, "records.query", contracts.ReadQuery{ID: r.URL.Query().Get("record")})
	if err != nil || len(result.Records) != 1 {
		reject(w, 401, "unauthorized")
		return
	}
	var contactPhoto struct {
		Derivatives map[string]json.RawMessage `json:"derivatives"`
	}
	json.Unmarshal(result.Records[0].Data, &contactPhoto)
	if result.Records[0].Collection == "contacts.contact" && (variant == "thumb" || variant == "display") && len(contactPhoto.Derivatives[variant]) == 0 {
		variant = "original"
	}
	var attachment struct {
		Blob struct {
			Digest string `json:"digest"`
		} `json:"blob"`
		MIME          string `json:"mime_type"`
		MIMEAlternate string `json:"mime"`
		Derivatives   map[string]struct {
			Digest        string `json:"digest"`
			MIME          string `json:"mime_type"`
			MIMEAlternate string `json:"mime"`
			State         string `json:"state"`
		} `json:"derivatives"`
	}
	json.Unmarshal(result.Records[0].Data, &attachment)
	mime := attachment.MIME
	if mime == "" {
		mime = attachment.MIMEAlternate
	}
	originalMIME := mime
	explicitVariant := variant != "original" && r.PathValue("digest") != attachment.Blob.Digest
	if explicitVariant && attachment.Derivatives[variant].State == "unavailable" {
		unavailableVariant(w, true)
		return
	}
	if variant != "original" {
		if derived, ok := attachment.Derivatives[variant]; ok && contracts.ValidDigest(derived.Digest) {
			if explicitVariant && derived.State == "pending" {
				unavailableVariant(w, false)
				return
			}

			digest = derived.Digest
			mime = derived.MIME
			if mime == "" {
				mime = derived.MIMEAlternate
			}
		} else if explicitVariant {
			unavailableVariant(w, false)
			return
		} else if !browserSafe(mime) {
			w.Header().Set("Retry-After", "2")
			reject(w, 409, "variant_pending")
			return
		}
	}
	if s.deps.Blobs == nil {
		reject(w, 503, "unavailable")
		return
	}
	file, err := s.deps.Blobs.OpenAuthorized(r.Context(), grant, digest)
	if err != nil && !explicitVariant && variant != "original" && browserSafe(originalMIME) && contracts.ValidDigest(attachment.Blob.Digest) {
		digest = attachment.Blob.Digest
		mime = originalMIME
		file, err = s.deps.Blobs.OpenAuthorized(r.Context(), grant, digest)
	}
	if err != nil {
		if explicitVariant {
			unavailableVariant(w, false)
			return
		}
		if variant != "original" {
			w.Header().Set("Retry-After", "2")
			reject(w, 409, "variant_pending")
		} else {
			reject(w, 404, "not_found")
		}
		return
	}
	defer file.Close()
	if mime == "" {
		head := make([]byte, 512)
		n, _ := file.Read(head)
		mime = http.DetectContentType(head[:n])
		file.Seek(0, io.SeekStart)
	}
	if !browserSafe(mime) {
		mime = "application/octet-stream"
		w.Header().Set("Content-Disposition", "attachment")
	}
	w.Header().Set("Content-Type", mime)
	w.Header().Set("ETag", `"`+digest+`"`)
	if digest == r.PathValue("digest") {
		w.Header().Set("Cache-Control", "private, max-age=31536000, immutable")
	} else {
		w.Header().Set("Cache-Control", "no-store")
	}
	http.ServeContent(w, r, "", time.Time{}, file)
}
func unavailableVariant(w http.ResponseWriter, permanent bool) {
	w.Header().Set("Cache-Control", "no-store")
	if permanent {
		reject(w, 404, "variant_unavailable")
		return
	}
	w.Header().Set("Retry-After", "2")
	reject(w, 503, "variant_pending")
}
func browserSafe(mime string) bool {
	switch strings.Split(mime, ";")[0] {
	case "image/jpeg", "image/png", "image/gif", "image/webp", "image/avif", "video/mp4", "video/webm", "audio/mp4", "audio/mpeg", "audio/aac", "audio/ogg", "audio/wav":
		return true
	}
	return false
}
