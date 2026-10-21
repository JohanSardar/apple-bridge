package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

func webMediaPlaybackScene(ctx context.Context, w *world.World, received bool) (map[string]string, error) {
	scene := simapple.Dataset{Blobs: map[string][]byte{}}
	reference := func(collection, key string) any {
		return map[string]any{"$ref": map[string]string{"collection": collection, "native_key": key}}
	}
	clips := map[string]struct {
		profile, caption string
		sent             bool
	}{
		"demo-message-00-000131": {"portrait", "A portrait from the synthetic studio.", false},
		"demo-message-00-000132": {"portrait", "A portrait for the synthetic studio.", true},
		"demo-message-00-000133": {"landscape", "A landscape for the synthetic studio.", true},
	}
	for _, record := range demo.Generate(demo.Options{Seed: 1}).Records {
		if record.Collection != "messages.message" {
			continue
		}
		spec, exists := clips[record.NativeKey]
		if !exists && record.NativeKey != "demo-message-00-000038" && record.NativeKey != "demo-message-00-000129" && record.NativeKey != "demo-message-00-000130" {
			continue
		}
		var fields map[string]any
		if err := json.Unmarshal(record.Data, &fields); err != nil {
			return nil, err
		}
		if record.NativeKey == "demo-message-00-000129" || record.NativeKey == "demo-message-00-000130" {
			if received {
				fields["is_from_me"], fields["sender_handle_id"] = false, reference("messages.handle", "demo-handle-01")
				delete(fields, "delivered_at")
				delete(fields, "read_at")
			}
			record.Data, _ = json.Marshal(fields)
			scene.Records = append(scene.Records, record)
			continue
		}
		if !exists {
			part := fields["parts"].([]any)[0].(map[string]any)
			part["index"] = 1
			fields["text"] = "A landscape from the synthetic studio."
			fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": fields["text"]}, part}
		} else {
			profile := "video"
			if spec.profile == "portrait" {
				profile = "video-portrait"
			}
			bytes := simapple.MediaFixture(profile)
			digest := fmt.Sprintf("%x", sha256.Sum256(bytes))
			scene.Blobs[digest] = bytes
			direction := "received"
			if spec.sent {
				direction = "sent"
			}
			key := "media-plate-" + spec.profile + "-" + direction
			attachment, _ := json.Marshal(map[string]any{"message_id": reference("messages.message", record.NativeKey), "filename": key + ".mp4", "mime_type": "video/mp4", "uti": "public.mpeg-4", "size": len(bytes), "blob": map[string]any{"digest": digest, "state": "present"}})
			scene.Records = append(scene.Records, contracts.Observation{Kind: "upsert", Collection: "messages.attachment", NativeKey: key, ScopedAlias: "messages.attachment/" + key, ObservedAt: demo.Epoch, Data: attachment})
			fields["is_from_me"], fields["text"] = spec.sent, spec.caption
			if spec.sent {
				fields["sender_handle_id"] = reference("messages.handle", "message-self")
				fields["delivered_at"] = fields["sent_at"]
			}
			fields["parts"] = []any{map[string]any{"index": 0, "kind": "text", "text": spec.caption}, map[string]any{"index": 1, "kind": "attachment", "attachment_id": reference("messages.attachment", key)}}
		}
		record.Data, _ = json.Marshal(fields)
		scene.Records = append(scene.Records, record)
	}
	if err := w.ControlAction("/dataset", scene); err != nil {
		return nil, err
	}
	return demo.Fixtures(ctx, w)
}
