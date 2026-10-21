package demo

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/simapple"
)

type Options struct {
	Seed int64
	Huge bool
}

type Dataset struct {
	simapple.Dataset
	Seed   int64          `json:"seed"`
	Counts map[string]int `json:"counts"`
}

func reference(collection, key string) any {
	return map[string]any{"$ref": map[string]string{"collection": collection, "native_key": key}}
}

func contactKey(i int) string { return fmt.Sprintf("demo-contact-%02d", i) }
func handleKey(i int) string {
	if i == 0 {
		return "message-self"
	}
	return fmt.Sprintf("demo-handle-%02d", i)
}
func chatKey(i int) string {
	if i == 0 {
		return "message-chat"
	}
	return fmt.Sprintf("demo-chat-%02d", i)
}
func messageKey(chat, index int) string { return fmt.Sprintf("demo-message-%02d-%06d", chat, index) }

func (d *Dataset) put(collection, key string, data map[string]any) {
	raw, _ := json.Marshal(data)
	d.Records = append(d.Records, contracts.Observation{Kind: "upsert", Collection: collection, NativeKey: key, ScopedAlias: collection + "/" + key, ObservedAt: Epoch, Data: raw})
	d.Counts[collection]++
}
func (d *Dataset) blob(data []byte) string {
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	d.Blobs[digest] = data
	return digest
}
func (d Dataset) Digest() string {
	raw, _ := json.Marshal(d)
	sum := sha256.Sum256(raw)
	return hex.EncodeToString(sum[:])
}
