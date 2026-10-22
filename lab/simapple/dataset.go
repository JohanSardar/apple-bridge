package simapple

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"net/http"
	"path/filepath"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type Dataset struct {
	Records    []contracts.Observation `json:"records"`
	Blobs      map[string][]byte       `json:"blobs"`
	LivePhotos map[string]string       `json:"live_photos,omitempty"`
}

func (f *fakeApple) datasetRoutes(mux *http.ServeMux) {
	mux.HandleFunc("POST /dataset", func(w http.ResponseWriter, r *http.Request) {
		var input Dataset
		if json.NewDecoder(http.MaxBytesReader(w, r.Body, 128<<20)).Decode(&input) != nil || len(input.Records) == 0 || len(input.Records) > 110000 {
			http.Error(w, "invalid dataset", 400)
			return
		}
		for digest, blob := range input.Blobs {
			sum := sha256.Sum256(blob)
			if hex.EncodeToString(sum[:]) != digest {
				http.Error(w, "invalid blob digest", 400)
				return
			}
		}
		for _, o := range input.Records {
			source := contracts.CollectionSource(o.Collection)
			if (source != contracts.Messages && source != contracts.Contacts && source != contracts.Reminders && source != contracts.Calendar) || o.NativeKey == "" || o.Kind != "upsert" || !json.Valid(o.Data) {
				http.Error(w, "invalid observation", 400)
				return
			}
		}
		pairs, valid := datasetLivePhotos(input)
		if !valid {
			http.Error(w, "invalid Live Photo pairing", 400)
			return
		}
		f.mu.Lock()
		for digest, blob := range input.Blobs {
			f.blobs[digest] = blob
		}
		captures := map[contracts.Source]contracts.Capture{}
		capturedAt := time.Now().UTC()
		variants := map[datasetMediaKey]datasetMedia{}
		for _, o := range input.Records {
			source := contracts.CollectionSource(o.Collection)
			if source == contracts.Reminders {
				o.ObservedAt = capturedAt
			}
			if source == contracts.Contacts {
				o.ScopedAlias = "contacts/" + o.Collection + "/" + o.NativeKey
				f.contactSequence++
			} else {
				o.ScopedAlias = o.Collection + "/" + o.NativeKey
			}
			if o.Collection == "messages.attachment" {
				var data struct {
					Filename string `json:"filename"`
					Blob     struct {
						Digest string `json:"digest"`
					} `json:"blob"`
				}
				json.Unmarshal(o.Data, &data)
				var fields map[string]any
				json.Unmarshal(o.Data, &fields)
				movie, live := pairs[o.NativeKey]
				key := datasetMediaKey{digest: data.Blob.Digest, extension: strings.ToLower(filepath.Ext(data.Filename)), live: live}
				if live {
					key.movie = sha256.Sum256(movie.data)
				}
				variant, exists := variants[key]
				if !exists {
					derivatives, media := f.mediaVariants(data.Filename, f.blobs[data.Blob.Digest])
					if live {
						clips, _ := f.mediaVariants(movie.filename, movie.data)
						derivatives["live"], media["live"] = clips["video"], true
						if poster := f.livePoster(f.blobs[data.Blob.Digest]); poster != nil {
							derivatives["poster"] = poster
						}
					}
					variant = datasetMedia{derivatives, media}
					variants[key] = variant
				}
				derivatives, media := variant.derivatives, variant.media
				fields["derivatives"], fields["media"] = derivatives, media
				o.Data, _ = json.Marshal(fields)
			}
			f.records[o.NativeKey] = o
			capture := captures[source]
			capture.Source = source
			capture.Observations = append(capture.Observations, o)
			captures[source] = capture
		}
		var persistErr error
		for _, source := range []contracts.Source{contracts.Contacts, contracts.Messages, contracts.Reminders, contracts.Calendar} {
			if _, exists := captures[source]; exists && persistErr == nil {
				persistErr = f.changedLocked(source)
			}
		}
		revisions := map[contracts.Source]uint64{}
		handles := map[string]string{}
		for source := range captures {
			revisions[source] = f.durable.Revision[source]
		}
		for digest := range f.blobs {
			handles[digest] = digest
		}
		f.mu.Unlock()
		if persistErr != nil {
			http.Error(w, "persist", 500)
			return
		}
		for _, source := range []contracts.Source{contracts.Contacts, contracts.Messages, contracts.Reminders, contracts.Calendar} {
			capture, exists := captures[source]
			if !exists {
				continue
			}
			capture.BlobHandles = handles
			if source == contracts.Contacts {
				capture.Generation = f.contactGeneration()
				capture.Checkpoint = f.contactToken()
			}
			if err := f.enqueue(r.Context(), capture, true, revisions[source]); err != nil {
				http.Error(w, "capture", 503)
				return
			}
		}
		json.NewEncoder(w).Encode(map[string]int{"records": len(input.Records)})
	})
}

type datasetMediaKey struct {
	digest    string
	extension string
	live      bool
	movie     [32]byte
}
type datasetMedia struct {
	derivatives map[string]any
	media       map[string]any
}
