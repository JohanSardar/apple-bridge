package simapple

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"image/png"
	"io"
	"os"
	"path/filepath"
)

func simulateVisionQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "vision.ocr" {
		return "", nil, false
	}
	var args struct {
		File      string   `json:"file"`
		Data      string   `json:"data_base64"`
		Languages []string `json:"languages"`
	}
	if contracts.DecodeInput(raw, &args) != nil {
		return "invalid", nil, true
	}
	data, err := base64.StdEncoding.DecodeString(args.Data)
	if args.File != "" {
		if args.Data != "" || !filepath.IsAbs(args.File) {
			return "invalid", nil, true
		}
		f, openErr := os.Open(args.File)
		if openErr != nil {
			return "unavailable", nil, true
		}
		defer f.Close()
		stat, statErr := f.Stat()
		if statErr != nil || !stat.Mode().IsRegular() || stat.Size() > 8<<20 {
			return "invalid", nil, true
		}
		data, err = io.ReadAll(io.LimitReader(f, (8<<20)+1))
	}
	if err != nil || len(data) == 0 || len(data) > 8<<20 {
		return "invalid", nil, true
	}
	if _, err := png.DecodeConfig(bytes.NewReader(data)); err != nil {
		return "invalid", nil, true
	}
	sum := sha256.Sum256(data)
	fixtures := map[string]struct{ Text, Language string }{
		"cd72477f797d20b7b3445289d36ddaac2be46d511dbb390612748d2462475e68": {"Bridge fixture", "en"},
		"7feee5a2404aaed713290b4fe33bf8320898dcebab51e100e0ed06ac59358fea": {"Bonjour le monde", "fr"},
		"53acf3e11bfeaf8d400cff84cf93861b6592da9573b206ab025e8b1a82063739": {"Guten Morgen", "de"},
	}
	fixture, ok := fixtures[hex.EncodeToString(sum[:])]
	if !ok {
		return "unavailable", nil, true
	}
	result, _ := json.Marshal(map[string]any{"text": fixture.Text, "language": fixture.Language, "lines": []any{map[string]any{"text": fixture.Text, "box": map[string]float64{"x": 0, "y": 0, "width": 1, "height": 1}}}})
	return "", result, true
}
