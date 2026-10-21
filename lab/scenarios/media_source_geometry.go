package scenarios

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"image"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func verifySourceVideoGeometry(ctx context.Context, e Evidence, w *world.World, chatID, prefix string) Evidence {
	add := func(name string, passed bool) {
		e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: passed})
	}
	names := []string{"portrait", "panorama", "fallback"}
	files := []contracts.MessageFile{}
	for _, name := range names {
		data := simapple.MediaFixture("video-" + name)
		if name == "fallback" {
			data = []byte("invalid synthetic video")
		}
		file, err := stageMessageFile(ctx, w.Gateway, prefix+"-geometry-"+name+".mov", data)
		if err != nil {
			add("source_geometry_stage_"+name, false)
			return e
		}
		files = append(files, file)
	}
	args, _ := json.Marshal(map[string]any{"chat_id": chatID, "text": "[ab-test " + e.RunID + "] source geometry", "files": files})
	client := scripted.Client{Endpoint: w.Gateway}
	command := contracts.Command{ID: contracts.NewCommandID(), Op: "messages.send", Args: args, WaitMS: 15000}
	var outcome contracts.CommandRecord
	add("source_geometry_observed", client.Call(ctx, command.Op, command, &outcome) == nil && outcome.State == contracts.CommandObserved)
	records := map[string]contracts.Record{}
	add("source_geometry_records", w.Await(10*time.Second, func() bool {
		var result contracts.ReadResult
		if client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.attachment", Limit: 100}, &result) != nil {
			return false
		}
		for _, record := range result.Records {
			var data mediaAttachment
			if json.Unmarshal(record.Data, &data) != nil {
				continue
			}
			for _, name := range names {
				if data.Filename == prefix+"-geometry-"+name+".mov" && data.Derivatives["video"].Digest != "" {
					records[name] = record
				}
			}
		}
		return len(records) == 3
	}) == nil)
	for _, name := range names {
		var data mediaAttachment
		json.Unmarshal(records[name].Data, &data)
		width, height, source := 180, 320, "source"
		if name == "panorama" {
			width, height = 1920, 200
		}
		if name == "fallback" {
			width, height, source = 16, 16, "sim-default"
		}
		add("source_geometry_"+name, data.Media.Width == width && data.Media.Height == height && data.Media.Duration == 1000 && data.Media.DimsSource == source)
		if name == "panorama" {
			variant := data.Derivatives["poster"]
			var status int
			var content []byte
			available := w.Await(15*time.Second, func() bool {
				var err error
				status, content, err = blobBytes(ctx, w.Gateway, variant.Digest, records[name].ID)
				return err == nil && status == 200
			}) == nil
			poster, _, decodeErr := image.Decode(bytes.NewReader(content))
			valid := available && decodeErr == nil
			size, pixel := image.Point{}, [3]uint32{}
			if valid {
				size = poster.Bounds().Size()
				r, g, b, _ := poster.At(size.X/2, size.Y/2).RGBA()
				pixel = [3]uint32{r, g, b}
				valid = size.X == 288 && size.Y == 30
			}
			if valid {
				r, g, b, _ := poster.At(144, 15).RGBA()
				valid = r > 50000 && g < 12000 && b < 12000
			}
			e.Assertions = append(e.Assertions, Assertion{Name: "source_geometry_center_marker_poster", Passed: valid, Detail: fmt.Sprintf("status=%d size=%dx%d rgb16=%v", status, size.X, size.Y, pixel)})
			video := data.Derivatives["video"]
			add("source_geometry_wide_decoded_frame", video.Width == 1920 && video.Height == 200)
		}
	}
	return e
}
