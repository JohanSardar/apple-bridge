package scenarios

import (
	"context"
	"encoding/json"
	"fmt"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func nativeMediaFixtureSimScenario(s Scenario) bool {
	switch s.ID {
	case "CALL-CAPTURE", "FILE-CAPTURE", "FILES-TRANSFORM", "VM-CAPTURE", "VMEMO-CAPTURE", "VMEMO-LATE-ASSETS", "NATIVE-MEDIA-EXPORT-SAFETY", "NATIVE-MEDIA-POSTERS", "NATIVE-MEDIA-YIELD":
		return true
	default:
		return false
	}
}

func verifyNativeMediaGateway(ctx context.Context, s Scenario, e Evidence) Evidence {
	add := func(name string, err error) bool {
		a := Assertion{Name: name, Passed: err == nil}
		if err != nil {
			a.Detail = err.Error()
		}
		e.Assertions = append(e.Assertions, a)
		e.Passed = e.Passed && a.Passed
		return a.Passed
	}
	w, err := world.New(ctx, "sim", 0, 0, world.BlobOptions{Scope: "admin"})
	if !add("selected_gateway_started", err) {
		return e
	}
	defer w.Close()
	source, collection := contracts.Source("messages"), "messages.attachment"
	fields := map[string]any{"filename": "[ab-test] native-media.txt", "mime_type": "text/plain", "uti": "public.plain-text", "size": 0, "blob": map[string]any{"state": "missing"}, "is_sticker": false, "hidden": false, "genmoji": false}
	switch s.ID {
	case "CALL-CAPTURE":
		source, collection = "phone", "phone.call"
		fields = map[string]any{"direction": "incoming", "handle": "synthetic", "name": "Fixture", "date": "2031-01-01T00:00:00Z", "duration": 42, "service": "cellular", "answered": true}
	case "FILE-CAPTURE", "FILES-TRANSFORM":
		source, collection = "files", "files.item"
		fields = map[string]any{"root": "/Bridge Test", "path": "fixture.txt", "name": "fixture.txt", "kind": "file", "size": 0, "created": nil, "modified": nil, "tags": []string{}, "downloaded": true, "icloud_status": "local"}
	case "VM-CAPTURE":
		source, collection = "voicemail", "voicemail.message"
		fields = map[string]any{"sender": "Synthetic", "date": "2031-01-01T00:00:00Z", "duration": 42, "read": false, "mailbox": "inbox", "type": "carrierVoicemail", "transcript_state": "missing", "audio_state": "missing"}
	case "VMEMO-CAPTURE", "VMEMO-LATE-ASSETS":
		source, collection = "voice_memos", "voice_memos.recording"
		fields = map[string]any{"title": "Synthetic", "date": "2031-01-01T00:00:00Z", "duration": 42, "folder": "1", "recently_deleted": false, "transcript_state": "missing", "audio_state": "missing"}
	}
	put := func(kind string, version int) error {
		fields["fixture_version"] = version
		data, err := json.Marshal(fields)
		if err != nil {
			return err
		}
		now := time.Now().UTC()
		capture := contracts.Capture{
			Source: source, Generation: "native-media-fixture",
			Observations: []contracts.Observation{{Kind: kind, Collection: collection, NativeKey: "native-media-fixture", Data: data, ObservedAt: now, Revalidated: true}},
		}
		if kind == "absent" {
			capture.ScanID = "native-media-fixture"
			capture.ScanStartedAt = now
			capture.Scope = []string{collection}
			capture.CompletionEvidence = "complete"
		}
		return w.ControlAction("/capture", capture)
	}
	var id string
	for version := 1; version <= 2; version++ {
		if !add(fmt.Sprintf("selected_gateway_capture_%d", version), put("upsert", version)) {
			return e
		}
		err := w.Await(5*time.Second, func() bool {
			var result contracts.ReadResult
			_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, Limit: 10}, &result)
			if err != nil || len(result.Records) != 1 {
				return false
			}
			r := result.Records[0]
			var data struct {
				Version int `json:"fixture_version"`
			}
			if json.Unmarshal(r.Data, &data) != nil || data.Version != version || r.Version != uint64(version) || id != "" && r.ID != id {
				return false
			}
			id = r.ID
			return true
		})
		if !add(fmt.Sprintf("selected_gateway_readback_%d", version), err) {
			return e
		}
	}
	if !add("selected_gateway_absent_capture", put("absent", 3)) {
		return e
	}
	add("selected_gateway_absent_readback", w.Await(5*time.Second, func() bool {
		var result contracts.ReadResult
		_, err := world.Request(ctx, w.Gateway, "/v1/op/records.query", contracts.ReadQuery{Collection: collection, Limit: 10}, &result)
		return err == nil && len(result.Records) == 0
	}))
	return e
}
