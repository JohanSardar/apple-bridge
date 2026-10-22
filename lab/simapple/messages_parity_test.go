package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"reflect"
	"testing"
	"time"
)

func TestMessageParityStateTransitions(t *testing.T) {
	f := &fakeApple{records: map[string]contracts.Observation{}}
	put := func(collection, key string, data map[string]any, absent bool) {
		raw, _ := json.Marshal(data)
		kind := "upsert"
		if absent {
			kind = "absent"
		}
		f.records[key] = contracts.Observation{Collection: "messages." + collection, NativeKey: key, Data: raw, Kind: kind}
	}
	read := func(key string) map[string]any {
		var d map[string]any
		json.Unmarshal(f.records[key].Data, &d)
		return d
	}
	for _, key := range []string{"a", "b", "c", "other"} {
		put("chat", key, map[string]any{"service": "iMessage"}, false)
	}
	index := 0
	action := contracts.NativeAction{Op: "messages.pin", NativeKey: "a", Messages: &contracts.NativeMessagesArgs{MessagesArgs: contracts.MessagesArgs{MessagesParityArgs: contracts.MessagesParityArgs{PinnedIndex: &index}}}}
	f.messageParityApply(action, time.Now(), put)
	before := f.messageBefore(action)
	action.NativeKey = "b"
	f.messageParityApply(action, time.Now(), put)
	action.NativeKey = "a"
	if reflect.DeepEqual(before, f.messageBefore(action)) {
		t.Fatal("another pin must invalidate prepared order")
	}
	if read("b")["pinned_index"] != float64(0) || read("a")["pinned_index"] != float64(1) {
		t.Fatal("ordered insert")
	}
	index = -1
	f.messageParityApply(action, time.Now(), put)
	if read("a")["pinned_index"] != nil || read("b")["pinned_index"] != float64(0) {
		t.Fatal("unpin compaction")
	}
	muted := true
	action.Op = "messages.mute"
	action.Messages.Muted = &muted
	f.messageParityApply(action, time.Now(), put)
	if read("a")["muted"] != true || read("a")["muted_until"] != nil {
		t.Fatal("permanent mute has no invented expiry")
	}
	action.Op = "messages.set_read_receipts"
	action.Messages.ReadReceipts = "off"
	f.messageParityApply(action, time.Now(), put)
	if read("a")["read_receipts"] != "off" {
		t.Fatal("receipt override")
	}
	for _, key := range []string{"a", "other"} {
		d := read(key)
		d["deleted_at"] = "2026-10-22T00:00:00Z"
		put("chat", key, d, false)
		put("message", key+"-message", map[string]any{"chat_id": messageReference("chat", key), "deleted_at": "2026-10-22T00:00:00Z"}, false)
		put("attachment", key+"-file", map[string]any{"message_id": messageReference("message", key+"-message")}, false)
	}
	action.Op = "messages.recover_chat"
	f.messageParityApply(action, time.Now(), put)
	if read("a")["deleted_at"] != nil || read("a-message")["deleted_at"] != nil || read("other")["deleted_at"] == nil {
		t.Fatal("recover exact conversation only")
	}
	if f.messageParityValidate(action, read("a")) == nil {
		t.Fatal("cannot recover active conversation")
	}
	d := read("a")
	d["deleted_at"] = "2026-10-22T00:00:00Z"
	put("chat", "a", d, false)
	d = read("a-message")
	d["deleted_at"] = "2026-10-22T00:00:00Z"
	put("message", "a-message", d, false)
	action.Op = "messages.delete_chat_permanently"
	f.messageParityApply(action, time.Now(), put)
	if f.records["a"].Kind != "absent" || f.records["a-message"].Kind != "absent" || f.records["other-message"].Kind == "absent" || f.records["a-file"].Kind != "absent" || f.records["other-file"].Kind == "absent" {
		t.Fatal("purge exact deleted membership only")
	}
}

func TestMessageParityPinLimit(t *testing.T) {
	f := &fakeApple{records: map[string]contracts.Observation{}}
	for i, key := range []string{"a", "b", "c", "d", "e", "f", "g", "h", "i", "new"} {
		d := map[string]any{"service": "iMessage"}
		if i < 9 {
			d["pinned_index"] = i
		}
		raw, _ := json.Marshal(d)
		f.records[key] = contracts.Observation{Collection: "messages.chat", NativeKey: key, Data: raw, Kind: "upsert"}
	}
	index := 0
	action := contracts.NativeAction{Op: "messages.pin", NativeKey: "new", Messages: &contracts.NativeMessagesArgs{MessagesArgs: contracts.MessagesArgs{MessagesParityArgs: contracts.MessagesParityArgs{PinnedIndex: &index}}}}
	var d map[string]any
	json.Unmarshal(f.records["new"].Data, &d)
	if f.messageParityValidate(action, d) != contracts.ErrConflict {
		t.Fatal("tenth pin rejected")
	}
	action.NativeKey = "a"
	json.Unmarshal(f.records["a"].Data, &d)
	if err := f.messageParityValidate(action, d); err != nil {
		t.Fatal("existing pin can reorder", err)
	}
}
