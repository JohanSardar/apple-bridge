package scenarios

import (
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func verifyDemoPosterResolution(records []contracts.Record, add func(string, bool)) {
	videos, livePhotos, goodVideos, goodLivePhotos := 0, 0, 0, 0
	for _, record := range records {
		var data mediaAttachment
		if json.Unmarshal(record.Data, &data) != nil {
			continue
		}
		video := strings.HasSuffix(data.Filename, ".mp4") || strings.HasSuffix(data.Filename, ".mov")
		if !video && !data.Media.Live {
			continue
		}
		poster := data.Derivatives["poster"]
		valid := contracts.ValidDigest(poster.Digest) && min(poster.Width, poster.Height) >= 360 && poster.Width*poster.Height <= 1500000
		if video {
			videos++
			if valid {
				goodVideos++
			}
		}
		if data.Media.Live {
			livePhotos++
			if valid {
				goodLivePhotos++
			}
		}
	}
	add("demo_video_posters_fit_retina_minimum_and_pixel_budget", videos > 0 && goodVideos == videos)
	add("demo_live_posters_fit_retina_minimum_and_pixel_budget", livePhotos > 0 && goodLivePhotos == livePhotos)
}
