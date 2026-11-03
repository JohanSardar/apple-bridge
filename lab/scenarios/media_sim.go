package scenarios

import (
	"bytes"
	"context"
	"database/sql"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"image"
	"image/color"
	"image/png"
	"path/filepath"
	"reflect"
	"strings"
	"time"
)

func runMediaVariantsSim(ctx context.Context, s Scenario, level string, e Evidence) Evidence {
	add := func(name string, ok bool) { e.Assertions = append(e.Assertions, Assertion{Name: name, Passed: ok}) }
	if s.ID == "WEB-MEDIA-VARIANTS" {
		proof := runMediaCleanupProof(ctx, s, level)
		for _, assertion := range proof.Assertions {
			assertion.Name = "cleanup_" + assertion.Name
			e.Assertions = append(e.Assertions, assertion)
		}
		if !proof.Passed {
			return e
		}
	}
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
	}{{prefix + ".png", buffer.Bytes()}, {prefix + ".mov", simapple.MediaFixture("video")}, {prefix + ".caf", simapple.MediaFixture("audio")}} {
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
	if s.ID == "WEB-MEDIA-VARIANTS" {
		e = verifySourceVideoGeometry(ctx, e, w, chats.Records[0].ID, prefix)
	}

	e.Passed = len(e.Assertions) > 0
	for _, a := range e.Assertions {
		e.Passed = e.Passed && a.Passed
	}
	return e
}
