package simapple

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"io"
	"os"
	"path/filepath"
)

func simulateTranscription(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "audio.transcribe" {
		return "", nil, false
	}
	var args struct {
		Data     string `json:"data_base64"`
		File     string `json:"file"`
		Language string `json:"language"`
	}
	if contracts.DecodeInput(raw, &args) != nil || len(args.Language) > 64 || (args.Data == "") == (args.File == "") {
		return "invalid", nil, true
	}
	data, err := base64.StdEncoding.DecodeString(args.Data)
	if args.File != "" {
		if !filepath.IsAbs(args.File) {
			return "invalid", nil, true
		}
		file, openErr := os.Open(args.File)
		if openErr != nil {
			return "unavailable", nil, true
		}
		defer file.Close()
		stat, statErr := file.Stat()
		if statErr != nil || !stat.Mode().IsRegular() || stat.Size() > 8<<20 {
			return "invalid", nil, true
		}
		data, err = io.ReadAll(io.LimitReader(file, (8<<20)+1))
	}
	if err != nil || len(data) == 0 || len(data) > 8<<20 {
		return "invalid", nil, true
	}
	_, fixture, _ := simulateAudioQuery("speech.synthesize", json.RawMessage(`{"text":"Bridge fixture"}`))
	var encoded struct {
		Data string `json:"data_base64"`
	}
	json.Unmarshal(fixture, &encoded)
	expected, _ := base64.StdEncoding.DecodeString(encoded.Data)
	if !bytes.Equal(data, expected) {
		return "unavailable", nil, true
	}
	return "", json.RawMessage(`{"status":"available","text":"Bridge fixture","language":"en-US","segments":[{"text":"Bridge fixture","start":0,"end":0.01}]}`), true
}
