package scenarios

import (
	"bytes"
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/png"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"strings"
	"sync"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type mediaAttachment struct {
	Filename string `json:"filename"`
	Blob     struct {
		Digest string `json:"digest"`
	} `json:"blob"`
	Derivatives map[string]struct {
		Digest string `json:"digest"`
		MIME   string `json:"mime"`
		Width  int    `json:"width"`
		Height int    `json:"height"`
		Handle string `json:"handle"`
	} `json:"derivatives"`
	Media struct {
		Width     int    `json:"width"`
		Height    int    `json:"height"`
		Duration  int    `json:"duration_ms"`
		ThumbHash string `json:"thumbhash"`
		Live      bool   `json:"live"`
		Waveform  []int  `json:"waveform"`
	} `json:"media"`
}

func runMediaVariants(ctx context.Context, s Scenario, level string) Evidence {
	sha, _ := exec.Command("git", "rev-parse", "HEAD").Output()
	e := Evidence{RunID: contracts.NewID("run"), Scenario: s.ID, Level: level, GitSHA: strings.TrimSpace(string(sha)), At: time.Now().UTC(), Latency: map[string]Latency{}, Hosts: []string{"air"}}
	if level != "sim" {
		return runNativeMessagesCommands(ctx, s, level, e)
	}
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	options := world.BlobOptions{Scope: "messages:read,messages:write,system:read"}
	if s.ID == "WEB-MEDIA-CAP" {
		options.Cap = 9 * 1024 * 1024
	}
	w, err := world.New(ctx, level, 1, 0, options)
	if err != nil {
		add("world_start", false)
		return e
	}
	defer w.Close()
	add("seed_self", w.ControlAction("/messages", map[string]any{"action": "seed", "count": 0}) == nil)
	client := scripted.Client{Endpoint: w.Gateway}
	var chats contracts.ReadResult
	add("self_chat_captured", w.Await(10*time.Second, func() bool {
		client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.chat", Limit: 100}, &chats)
		return len(chats.Records) > 0
	}) == nil)
	if len(chats.Records) == 0 {
		return e
	}
	picture := image.NewNRGBA(image.Rect(0, 0, 2400, 1200))
	noise := uint32(1)
	for y := 0; y < 1200; y++ {
		for x := 0; x < 2400; x++ {
			pixel := color.NRGBA{R: uint8(x % 256), G: uint8(y % 256), B: 90, A: 255}
			if s.ID == "WEB-MEDIA-CAP" {
				noise = noise*1664525 + 1013904223
				pixel.R = uint8(noise >> 24)
				noise = noise*1664525 + 1013904223
				pixel.G = uint8(noise >> 24)
				noise = noise*1664525 + 1013904223
				pixel.B = uint8(noise >> 24)
			}
			picture.SetNRGBA(x, y, pixel)
		}
	}
	var buffer bytes.Buffer
	png.Encode(&buffer, picture)
	files := []contracts.MessageFile{}
	prefix := "ab-" + e.RunID
	for _, fixture := range []struct {
		name  string
		bytes []byte
	}{{prefix + ".png", buffer.Bytes()}, {prefix + ".mov", []byte("synthetic video source")}, {prefix + ".caf", []byte("synthetic audio source")}} {
		file, err := stageMessageFile(ctx, w.Gateway, fixture.name, fixture.bytes)
		if err != nil {
			add("synthetic_upload", false)
			return e
		}
		files = append(files, file)
	}
	args, _ := json.Marshal(map[string]any{"chat_id": chats.Records[0].ID, "text": "[ab-test " + e.RunID + "] synthetic media", "files": files})
	observers := []*watcher{}
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		observer, err := watch(ctx, endpoint, false, func(contracts.Record) bool { return true })
		if err != nil {
			add("media_observer", false)
			return e
		}
		observers = append(observers, observer)
		defer observer.close()
	}
	started := time.Now()
	var outcome contracts.CommandRecord
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
	err = client.Call(ctx, command.Op, command, &outcome)
	add("self_send_observed", err == nil && outcome.State == contracts.CommandObserved)
	e = verifyMediaVariants(ctx, e, w, prefix, func(string) time.Time { return started }, observers)
	if s.ID == "WEB-MEDIA-CAP" {
		add("variants_fit_cap_original_deferred", w.Await(10*time.Second, func() bool {
			var records contracts.ReadResult
			if client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.attachment", Limit: 100}, &records) != nil {
				return false
			}
			progress, err := blobProgress(ctx, w.Replicas[0])
			if err != nil || progress.Bytes > options.Cap {
				return false
			}
			for _, record := range records.Records {
				var data mediaAttachment
				json.Unmarshal(record.Data, &data)
				if strings.HasSuffix(data.Filename, ".png") {
					return progress.Entries[data.Blob.Digest].State == "over_cap"
				}
			}
			return false
		}) == nil)
	}
	if s.ID == "WEB-MEDIA-CAP" {
		progress, _ := blobProgress(ctx, w.Replicas[0])
		if e.Metrics == nil {
			e.Metrics = map[string]any{}
		}
		e.Metrics["cap_entries"] = progress.Entries
		db, problem := sql.Open("sqlite", "file:"+filepath.Join(w.Replicas[0].Home, "state.db")+"?mode=ro")
		if problem == nil {
			defer db.Close()
			refs := []contracts.BlobReference{}
			rows, problem := db.QueryContext(ctx, "SELECT payload FROM blob_refs")
			if problem == nil {
				for rows.Next() {
					var raw []byte
					var ref contracts.BlobReference
					if rows.Scan(&raw) == nil && json.Unmarshal(raw, &ref) == nil {
						refs = append(refs, ref)
					}
				}
				rows.Close()
			}
			e.Metrics["cap_references"] = refs
		}
	}
	before := mediaSnapshot(ctx, w.Gateway, prefix)
	add("sim_restart", w.RestartHelper() == nil)
	add("variants_survive_restart", w.Await(15*time.Second, func() bool {
		return len(before) == 3 && reflect.DeepEqual(before, mediaSnapshot(ctx, w.Gateway, prefix))
	}) == nil)
	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}

func mediaTool() string {
	data, _ := os.ReadFile("native/.build/native-output")
	return filepath.Join(strings.TrimSpace(string(data)), "MediaChecks")
}

func prepareMediaFiles(ctx context.Context, w *world.World, run string) ([]contracts.MessageFile, error) {
	directory := filepath.Join(w.Dir, "synthetic-media")
	if _, err := exec.CommandContext(ctx, mediaTool(), "fixtures", directory, run).Output(); err != nil {
		return nil, err
	}
	files := []contracts.MessageFile{}
	for _, extension := range []string{"heic", "mov", "caf"} {
		name := "ab-" + run + "." + extension
		data, err := os.ReadFile(filepath.Join(directory, name))
		if err != nil {
			return nil, err
		}
		file, err := stageMessageFile(ctx, w.Gateway, name, data)
		if err != nil {
			return nil, err
		}
		files = append(files, file)
	}
	return files, nil
}

func observeMediaAppearance(ctx context.Context, db *sql.DB, run string) func(string) time.Time {
	var mu sync.Mutex
	seen := map[string]time.Time{}
	go func() {
		ticker := time.NewTicker(10 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-ctx.Done():
				return
			case <-ticker.C:
				rows, err := db.QueryContext(ctx, "SELECT a.filename FROM attachment a JOIN message_attachment_join j ON j.attachment_id=a.ROWID JOIN message m ON m.ROWID=j.message_id WHERE m.is_from_me=1 AND a.filename LIKE ?", "%ab-"+run+".%")
				if err != nil {
					continue
				}
				for rows.Next() {
					var name string
					if rows.Scan(&name) == nil {
						mu.Lock()
						name = filepath.Base(name)
						if seen[name].IsZero() {
							seen[name] = time.Now()
						}
						mu.Unlock()
					}
				}
				rows.Close()
			}
		}
	}()
	return func(name string) time.Time { mu.Lock(); defer mu.Unlock(); return seen[name] }
}

func verifyMediaVariants(ctx context.Context, e Evidence, w *world.World, prefix string, appeared func(string) time.Time, observers []*watcher) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	endpoints := append([]world.Endpoint{w.Gateway}, w.Replicas...)
	for index, endpoint := range endpoints {
		found := map[string]contracts.Record{}
		seen := map[string]time.Time{}
		ready := w.Await(45*time.Second, func() bool {
			var result contracts.ReadResult
			if (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.attachment", Limit: 10000}, &result) != nil {
				return false
			}
			for _, record := range result.Records {
				var data mediaAttachment
				if json.Unmarshal(record.Data, &data) != nil || !strings.Contains(data.Filename, prefix+".") {
					continue
				}
				wanted := []string{"thumb", "display"}
				if e.Level != "sim" && strings.HasSuffix(data.Filename, ".heic") {
					wanted = append(wanted, "live")
				}
				if strings.HasSuffix(data.Filename, ".mov") {
					wanted = []string{"video", "poster"}
				} else if strings.HasSuffix(data.Filename, ".caf") {
					wanted = []string{"audio"}
				}
				complete := true
				for _, name := range wanted {
					if !contracts.ValidDigest(data.Derivatives[name].Digest) {
						complete = false
					}
				}
				if complete {
					found[data.Filename] = record
					if seen[data.Filename].IsZero() {
						seen[data.Filename] = time.Now()
						if index < len(observers) {
							if at := mediaVariantAt(observers[index], data.Filename); !at.IsZero() {
								seen[data.Filename] = at
							}
						}
					}
				}
			}
			return len(found) == 3
		}) == nil
		add(fmt.Sprintf("variants_captured_%d", index), ready)
		for name, record := range found {
			var data mediaAttachment
			json.Unmarshal(record.Data, &data)
			kind := "image"
			if strings.HasSuffix(name, ".mov") {
				kind = "video"
			}
			if strings.HasSuffix(name, ".caf") {
				kind = "audio"
			}
			start := appeared(name)
			add(kind+"_native_appearance_measured", !start.IsZero())
			if !start.IsZero() {
				elapsed := max(0, float64(seen[name].Sub(start))/float64(time.Millisecond))
				e.Latency[fmt.Sprintf("%s_appearance_to_variants_%d", kind, index)] = stats([]float64{elapsed}, 0)
				if kind == "image" && index == 0 {
					add("image_variants_within_2_seconds", elapsed <= 2000)
				}
			}
			add(kind+"_original_preserved", contracts.ValidDigest(data.Blob.Digest))
			if kind == "image" && e.Level != "sim" {
				add("live_photo_identifier_pair", data.Media.Live && data.Derivatives["live"].MIME == "video/mp4")
			}
			if kind == "audio" {
				add("audio_duration_and_64_bounded_peaks", data.Media.Duration >= 950 && data.Media.Duration <= 1100 && len(data.Media.Waveform) == 64)
				for _, peak := range data.Media.Waveform {
					if peak < 0 || peak > 255 {
						add("waveform_range", false)
					}
				}
			} else {
				add(kind+"_dimensions_and_thumbhash", data.Media.Width > 0 && data.Media.Height > 0 && data.Media.ThumbHash != "")
			}
			for variantName, variant := range data.Derivatives {
				add("private_handles_removed", variant.Handle == "")
				var content []byte
				available := w.Await(15*time.Second, func() bool {
					status, b, err := blobBytes(ctx, endpoint, variant.Digest, record.ID)
					if status == 200 && err == nil {
						content = b
						return true
					}
					return false
				}) == nil
				add(fmt.Sprintf("%s_authorized_blob_%d", variantName, index), available)
				if !available {
					continue
				}
				add("variant_digest_verified", fmt.Sprintf("%x", sha256.Sum256(content)) == variant.Digest)
				if strings.HasPrefix(variant.MIME, "image/") {
					config, format, err := image.DecodeConfig(bytes.NewReader(content))
					add(variantName+"_browser_image", err == nil && (format == "jpeg" || format == "png") && config.Width == variant.Width && config.Height == variant.Height)
					maximum := 1600
					if variantName == "thumb" || variantName == "poster" {
						maximum = 400
					}
					add(variantName+"_dimensions_bounded", max(variant.Width, variant.Height) <= maximum)
				}
				if e.Level != "sim" {
					extension := ".m4a"
					if variant.MIME == "video/mp4" {
						extension = ".mp4"
					}
					if variant.MIME == "image/jpeg" {
						extension = ".jpg"
					}
					if variant.MIME == "image/png" {
						extension = ".png"
					}
					file := filepath.Join(w.Dir, "inspect-"+variant.Digest+extension)
					os.WriteFile(file, content, 0600)
					observation, err := w.Truth("observe", map[string]any{"source": "media", "file": file})
					output, encodingErr := json.Marshal(observation)
					var truth struct {
						Width    int      `json:"width"`
						Height   int      `json:"height"`
						Duration float64  `json:"duration_ms"`
						Video    []string `json:"video"`
						Audio    []string `json:"audio"`
						Type     string   `json:"type"`
					}
					parsed := err == nil && encodingErr == nil && json.Unmarshal(output, &truth) == nil
					add(variantName+"_avfoundation_truth", parsed)
					if variant.MIME == "video/mp4" {
						add("mp4_h264_1080p_faststart", parsed && len(truth.Video) > 0 && truth.Video[0] == "avc1" && min(truth.Width, truth.Height) <= 1080 && max(truth.Width, truth.Height) <= 1920 && truth.Duration >= 950 && truth.Duration <= 1100 && bytes.Index(content, []byte("moov")) < bytes.Index(content, []byte("mdat")))
					}
					if variantName == "audio" {
						add("audio_aac_truth", parsed && len(truth.Audio) > 0 && truth.Audio[0] == "aac " && truth.Duration >= 950 && truth.Duration <= 1100)
					}
					if strings.HasPrefix(variant.MIME, "image/") {
						add("imageio_jpeg_png_truth", parsed && (truth.Type == "public.jpeg" || truth.Type == "public.png") && truth.Width == variant.Width && truth.Height == variant.Height)
					}
					os.Remove(file)
				}
			}
		}
	}
	if e.Metrics == nil {
		e.Metrics = map[string]any{}
	}
	e.Metrics["browser_playback"] = "Cross-engine playback is owned by the later UI unit"
	return e
}

func mediaVariantAt(observer *watcher, name string) time.Time {
	observer.mu.Lock()
	defer observer.mu.Unlock()
	for _, change := range observer.changes {
		if change.Change.Record.Collection != "messages.attachment" {
			continue
		}
		var data mediaAttachment
		if json.Unmarshal(change.Change.Record.Data, &data) != nil || data.Filename != name {
			continue
		}
		wanted := []string{"thumb", "display"}
		if strings.HasSuffix(name, ".mov") {
			wanted = []string{"video", "poster"}
		}
		if strings.HasSuffix(name, ".caf") {
			wanted = []string{"audio"}
		}
		ready := true
		for _, variant := range wanted {
			if !contracts.ValidDigest(data.Derivatives[variant].Digest) {
				ready = false
			}
		}
		if ready {
			return change.At
		}
	}
	return time.Time{}
}

func mediaSnapshot(ctx context.Context, endpoint world.Endpoint, prefix string) map[string]string {
	var result contracts.ReadResult
	if (scripted.Client{Endpoint: endpoint}).Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.attachment", Limit: 10000}, &result) != nil {
		return nil
	}
	snapshot := map[string]string{}
	for _, record := range result.Records {
		var data mediaAttachment
		if json.Unmarshal(record.Data, &data) == nil && strings.Contains(data.Filename, prefix+".") {
			canonical, _ := json.Marshal(data)
			snapshot[data.Filename] = string(canonical)
		}
	}
	return snapshot
}
