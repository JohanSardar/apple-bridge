package simapple

import (
	"encoding/json"
	"strings"
)

type livePhotoAsset struct {
	filename string
	data     []byte
}

func datasetLivePhotos(input Dataset) (map[string]livePhotoAsset, bool) {
	pairs := map[string]livePhotoAsset{}
	if len(input.LivePhotos) == 0 {
		return pairs, true
	}
	if len(input.LivePhotos) > len(input.Records) {
		return nil, false
	}
	assets := map[string]livePhotoAsset{}
	for _, record := range input.Records {
		if record.Collection != "messages.attachment" {
			continue
		}
		var fields struct {
			Filename string `json:"filename"`
			Blob     struct {
				Digest string `json:"digest"`
			} `json:"blob"`
		}
		if json.Unmarshal(record.Data, &fields) != nil {
			return nil, false
		}
		assets[record.NativeKey] = livePhotoAsset{fields.Filename, input.Blobs[fields.Blob.Digest]}
	}
	for still, companion := range input.LivePhotos {
		image, first := assets[still]
		movie, second := assets[companion]
		if !first || !second || len(image.data) == 0 || len(movie.data) == 0 || !strings.HasPrefix(simMediaMIME(image.filename, image.data), "image/") || !strings.HasPrefix(simMediaMIME(movie.filename, movie.data), "video/") {
			return nil, false
		}
		pairs[still] = movie
	}
	return pairs, true
}
