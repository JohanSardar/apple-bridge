package scenarios

import (
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"strings"
	"time"
)

func demoItemCoverage(item contracts.ThreadItem, c map[string]int) {
	d := rmData(item.Record)
	text, _ := d["text"].(string)
	if text != "" {
		c["text"]++
	}
	if len(text) > 2000 {
		c["long_text"]++
	}
	for i, emoji := range []string{"🌿", "🌿✨", "🌿✨🪁"} {
		if text == emoji {
			c[fmt.Sprintf("emoji_%d", i+1)]++
		}
	}
	if strings.Contains(text, "https://example.com") {
		c["url"]++
	}
	if d["reply_to_id"] != nil {
		c["reply"]++
		if d["reply_to_id"] != d["thread_originator_id"] {
			c["nested_reply"]++
		}
	}
	if item.ReplyCount >= 4 {
		c["reply_count"]++
	}
	if d["edited_at"] != nil && len(d["edit_history"].([]any)) >= 1 {
		c["edited"]++
	}
	if parts, ok := d["retracted_parts"].([]any); ok && len(parts) > 0 {
		c["unsent"]++
	}
	if d["error_code"].(float64) != 0 {
		c["failed"]++
	}
	if d["delivered_at"] != nil {
		c["delivered"]++
	}
	if d["read_at"] != nil {
		c["read"]++
	}
	if effect, ok := d["effect"].(string); ok {
		c["effect_"+effect]++
	}
	if event, ok := d["group_event"].(map[string]any); ok {
		c["group_"+fmt.Sprint(event["kind"])]++
	}
	if parts, ok := d["parts"].([]any); ok {
		if len(parts) > 1 {
			c["multipart"]++
		}
		for _, part := range parts {
			if part.(map[string]any)["mention"] != nil {
				c["mention"]++
			}
		}
	}
	at, _ := time.Parse(time.RFC3339, fmt.Sprint(d["sent_at"]))
	days := int(demo.Epoch.Sub(at).Hours() / 24)
	switch {
	case days == 0:
		c["today"]++
	case days == 1:
		c["yesterday"]++
	case days < 7:
		c["this_week"]++
	default:
		c["older"]++
	}
	if at.Month() == time.December && at.Day() == 31 || at.Month() == time.January && at.Day() == 1 {
		c["year_change"]++
	}
	for _, reaction := range item.Reactions {
		fields := rmData(reaction)
		c["reaction_"+fmt.Sprint(fields["kind"])]++
		if fields["removed"] == true {
			c["removed_reaction"]++
		}
	}
	if len(item.Reactions) >= 3 {
		c["multiple_reactors"]++
	}
	for _, attachment := range item.Attachments {
		fields := rmData(attachment)
		filename := fmt.Sprint(fields["filename"])
		dimensions := map[string][2]int{"demo-portrait.png": {240, 400}, "demo-landscape.png": {640, 400}, "demo-square.png": {360, 360}, "demo-panorama.png": {1024, 256}, "demo-transparent.png": {256, 256}, "demo-video.mp4": {320, 180}, "demo-ultrawide.mp4": {1920, 200}}
		if want, exists := dimensions[filename]; exists {
			metadata, _ := fields["media"].(map[string]any)
			if metadata["width"] == float64(want[0]) && metadata["height"] == float64(want[1]) {
				c["geometry_"+filename]++
			}
		}

		for _, kind := range []string{"portrait", "landscape", "square", "panorama", "transparent"} {
			if strings.Contains(filename, kind) {
				c[kind]++
			}
		}
		for ext, kind := range map[string]string{".gif": "gif", ".mp4": "video", ".m4a": "audio", ".pdf": "pdf", ".vcf": "vcard"} {
			if strings.HasSuffix(filename, ext) {
				c[kind]++
			}
		}
		derivatives, _ := fields["derivatives"].(map[string]any)
		if strings.HasSuffix(filename, ".mp4") && derivatives["video"] != nil && derivatives["poster"] != nil {
			c["video_variants"]++
		}
		if strings.HasSuffix(filename, ".m4a") && derivatives["audio"] != nil {
			c["audio_variants"]++
		}
		if strings.HasSuffix(filename, ".gif") && derivatives["thumb"] != nil {
			c["gif_thumb"]++
		}
		if filename == "demo-transparent.png" {
			if display, ok := derivatives["display"].(map[string]any); ok && display["mime"] == "image/png" {
				c["transparent_display_png"]++
			}
		}
		if metadata, ok := fields["media"].(map[string]any); ok {
			if waveform, ok := metadata["waveform"].([]any); ok && len(waveform) == 64 {
				c["audio_waveform"]++
			}
		}
		if strings.HasPrefix(fmt.Sprint(fields["mime_type"]), "image/") && len(derivatives) > 0 {
			c["image_variants"]++
		}
		if filename == "demo-portrait.png" || filename == "demo-landscape.png" || filename == "demo-square.png" {
			c["image_stack"]++
		}
	}
}
