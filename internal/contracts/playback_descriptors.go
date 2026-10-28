package contracts

import (
	"encoding/json"
	"slices"
	"strings"
)

type PlaybackArgs struct {
	Text   string   `json:"text,omitempty"`
	Name   string   `json:"name,omitempty"`
	Volume *float64 `json:"volume,omitempty"`
}

func PlaybackOperation(op string) bool { return op == "speech.say" || op == "capture.play_sound" }
func ValidatePlayback(c Command) error {
	var a PlaybackArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil || a.Volume != nil && (*a.Volume < 0 || *a.Volume > 1) {
		return ErrInvalid
	}
	if c.Op == "speech.say" {
		if strings.TrimSpace(a.Text) == "" || len(a.Text) > 2048 || a.Name != "" {
			return ErrInvalid
		}
	} else if a.Text != "" || !slices.Contains([]string{"Basso", "Blow", "Bottle", "Frog", "Funk", "Glass", "Hero", "Morse", "Ping", "Pop", "Purr", "Sosumi", "Submarine", "Tink"}, a.Name) || a.Name == "" {
		return ErrInvalid
	}
	return nil
}
func PlaybackEvidence(raw json.RawMessage, hash string) bool {
	var value struct {
		Finished bool `json:"finished_callback"`
		Prepared struct {
			Hash string `json:"request_hash"`
		} `json:"prepared"`
	}
	return json.Unmarshal(raw, &value) == nil && value.Finished && hash != "" && value.Prepared.Hash == hash
}
func init() {
	RegisterSource("capture", "Capture", true)
	registerDescriptors(func() []OperationDescriptor {
		return []OperationDescriptor{
			{Name: "speech.say", Version: 1, Kind: CommandOp, Source: "speech", Scope: "speech:write", Capability: "speech.say", Input: Command{}, Output: CommandRecord{}, Description: "Speak text with finished callback evidence"},
			{Name: "capture.play_sound", Version: 1, Kind: CommandOp, Source: "capture", Scope: "capture:write", Capability: "capture.play_sound", Input: Command{}, Output: CommandRecord{}, Description: "Play a named system sound with finished callback evidence"},
		}
	}, 76, 77)
}
