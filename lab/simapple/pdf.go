package simapple

import (
	"bytes"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"io"
	"os"
	"path/filepath"
	"strings"
)

func (f *fakeApple) simulatePDFQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "pdf.text" && op != "pdf.search" {
		return "", nil, false
	}
	var args struct {
		File string `json:"file"`
		Data string `json:"data_base64"`
		Text string `json:"text"`
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
	if err != nil || len(data) > 8<<20 || !bytes.HasPrefix(data, []byte("%PDF-")) || (op == "pdf.search" && args.Text == "") {
		return "invalid", nil, true
	}
	sum := sha256.Sum256(data)
	digest := hex.EncodeToString(sum[:])
	if digest != "f71a9207e47bd6526a9a5a346541df4eb84de43649bcb11fce1ba7aa971b07cc" {
		known := false
		f.mu.Lock()
		for _, record := range f.records {
			if record.Collection == "pdf.artifact" {
				for _, ref := range contracts.BlobReferences(record.Data) {
					if ref.Digest == digest {
						known = true
					}
				}
			}
		}
		f.mu.Unlock()
		if !known {
			return "unavailable", nil, true
		}
	}
	if op == "pdf.text" {
		return "", json.RawMessage(`{"pages":[{"page":1,"text":"Bridge fixture"}],"page_count":1}`), true
	}
	matches := []any{}
	if offset := strings.Index(strings.ToLower("Bridge fixture"), strings.ToLower(args.Text)); offset >= 0 {
		matches = append(matches, map[string]int{"page": 1, "offset": offset, "length": len(args.Text)})
	}
	result, _ := json.Marshal(map[string]any{"matches": matches, "page_count": 1})
	return "", result, true
}
