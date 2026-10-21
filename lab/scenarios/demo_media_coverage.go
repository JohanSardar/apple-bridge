package scenarios

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/demo"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
	"sort"
	"strings"
	"time"
)

func demoMediaCoverage(ctx context.Context, w *world.World, dataset demo.Dataset, mediaRecords []contracts.Record, coverage map[string]int, e *Evidence, add func(string, bool)) {
	client := scripted.Client{Endpoint: w.Gateway, Protocol: "rest"}
	var err error
	observations, inspectErr := w.Inspect()
	raw, _ := json.Marshal(observations)
	add("captured_records_secret_scan", inspectErr == nil && demoSynthetic(raw, nil))
	removed := 0
	for _, o := range observations {
		var fields map[string]any
		json.Unmarshal(o.Data, &fields)
		if o.Collection == "messages.reaction" && fields["removed"] == true {
			removed++
		}
	}
	add("removed_reaction_captured_and_folded_out", removed == 1 && coverage["removed_reaction"] == 0)
	var contacts contracts.ReadResult
	err = client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "contacts.contact", Limit: 200}, &contacts)
	photos := 0
	for _, contact := range contacts.Records {
		if rmData(contact)["image"] != nil {
			photos++
		}
	}
	add("contacts_30_generated_avatars_30_monograms", err == nil && len(contacts.Records) == 60 && photos == 30)
	var chats contracts.ChatsResult
	client.Call(ctx, "messages.chats", contracts.ChatsQuery{Limit: 200}, &chats)
	summaries := chats.Chats
	linked := 0
	for _, chat := range summaries {
		for _, avatar := range chat.AvatarHandles {
			if avatar.ContactID != nil {
				linked++
			}
		}
	}
	add("contacts_linked_to_chat_avatars", linked > 30)
	mediaRecords = append(mediaRecords, contacts.Records...)
	checked := map[string]bool{}
	blobsOK := true
	for _, record := range mediaRecords {
		for _, ref := range contracts.BlobReferences(record.Data) {
			if checked[ref.Digest] {
				continue
			}
			available := w.Await(5*time.Second, func() bool {
				status, content, err := blobBytes(ctx, w.Gateway, ref.Digest, record.ID)
				return err == nil && status == 200 && fmt.Sprintf("%x", sha256.Sum256(content)) == ref.Digest
			}) == nil
			blobsOK = blobsOK && available
			checked[ref.Digest] = true
		}
	}
	digests := make([]string, 0, len(checked))
	for digest := range checked {
		digests = append(digests, digest)
	}
	sort.Strings(digests)
	e.Metrics["verified_blob_digests"] = digests
	e.Metrics["verified_blob_count"] = len(checked)
	add("generated_originals_avatars_and_sim_variants_download_and_hash", blobsOK && len(checked) > 50)
	before := demoObservationDigest(observations)
	err = demo.Populate(w, dataset)
	again, againErr := w.Inspect()
	add("capture_records_and_variant_digests_repeat_identically", err == nil && againErr == nil && before == demoObservationDigest(again))
	liveContext, cancel := context.WithCancel(ctx)
	done := make(chan error, 1)
	go func() { done <- demo.Alive(liveContext, w, 50*time.Millisecond) }()
	liveErr := w.Await(3*time.Second, func() bool {
		observations, err := w.Inspect()
		if err != nil {
			return false
		}
		kinds := map[string]bool{}
		for _, o := range observations {
			if strings.HasPrefix(o.NativeKey, "demo-live-") {
				kinds[o.Collection] = true
			}
		}
		return kinds["messages.message"] && kinds["messages.reaction"]
	})
	cancel()
	aliveErr := <-done
	add("alive_messages_and_reactions_captured", liveErr == nil && aliveErr == nil)
}

func demoObservationDigest(observations []contracts.Observation) string {
	filtered := []contracts.Observation{}
	for _, o := range observations {
		if contracts.CollectionSource(o.Collection) == contracts.Messages || contracts.CollectionSource(o.Collection) == contracts.Contacts {
			filtered = append(filtered, o)
		}
	}
	sort.Slice(filtered, func(i, j int) bool { return filtered[i].NativeKey < filtered[j].NativeKey })
	raw, _ := json.Marshal(filtered)
	return fmt.Sprintf("%x", sha256.Sum256(raw))
}
