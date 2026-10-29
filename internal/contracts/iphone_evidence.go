package contracts

import (
	"crypto/ed25519"
	"encoding/json"
	"strconv"
)

func IPhoneCompletionEvidence(raw []byte, command Command, hash string) bool {
	var receipt struct {
		DeviceID   string `json:"device_id"`
		At         int64  `json:"at"`
		Payload    []byte `json:"payload"`
		Signature  []byte `json:"signature"`
		SigningKey []byte `json:"signing_key"`
	}
	var result struct {
		ID    string `json:"command_id"`
		Hash  string `json:"request_hash"`
		State string `json:"state"`
	}
	var target IPhoneTarget
	if json.Unmarshal(raw, &receipt) != nil || json.Unmarshal(receipt.Payload, &result) != nil || json.Unmarshal(command.Args, &target) != nil || result.ID != command.ID || result.Hash != hash || result.State != "observed" || receipt.DeviceID != target.DeviceID || len(receipt.SigningKey) != ed25519.PublicKeySize {
		return false
	}
	message := append([]byte("apple-bridge-iphone/1\x00/v1/iphone/result\x00"+strconv.FormatInt(receipt.At, 10)+"\x00"), receipt.Payload...)
	return ed25519.Verify(receipt.SigningKey, message, receipt.Signature)
}
