package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/lab/scripted"
	"github.com/JohanSardar/apple-bridge/lab/world"
)

type attachment struct {
	Filename    string             `json:"filename"`
	Derivatives map[string]variant `json:"derivatives"`
	Media       map[string]any     `json:"media"`
}
type variant struct {
	Digest string `json:"digest"`
	MIME   string `json:"mime"`
	Width  int    `json:"width"`
	Height int    `json:"height"`
}

func run() error {
	if len(os.Args) != 2 || !regexp.MustCompile(`^run_[A-Z0-9]+$`).MatchString(os.Args[1]) {
		return fmt.Errorf("expected one existing synthetic media run id")
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	if err := world.Build(ctx, "replica"); err != nil {
		return err
	}
	w, err := world.NewReal(ctx, "replica", 1, world.BlobOptions{Scope: "messages:read,system:read"})
	if err != nil {
		return err
	}
	defer w.Close()
	prefix := "ab-" + os.Args[1] + "."
	output := []map[string]any{}
	for _, endpoint := range append([]world.Endpoint{w.Gateway}, w.Replicas...) {
		client := scripted.Client{Endpoint: endpoint}
		records := []contracts.Record{}
		if err := w.Await(45*time.Second, func() bool {
			records = nil
			after := ""
			for {
				var page contracts.ReadResult
				if client.Call(ctx, "records.query", contracts.ReadQuery{Collection: "messages.attachment", Limit: 1000, After: after}, &page) != nil {
					return false
				}
				for _, r := range page.Records {
					var a attachment
					if json.Unmarshal(r.Data, &a) == nil && strings.Contains(a.Filename, prefix) && len(a.Derivatives) > 0 {
						records = append(records, r)
					}
				}
				if page.Next == "" {
					break
				}
				after = page.Next
			}
			return len(records) == 3
		}); err != nil {
			return fmt.Errorf("existing synthetic attachments not captured")
		}
		for _, record := range records {
			var a attachment
			if err := json.Unmarshal(record.Data, &a); err != nil {
				return err
			}
			wanted := []string{"thumb", "display", "live"}
			if strings.HasSuffix(a.Filename, ".mov") {
				wanted = []string{"video", "poster"}
			}
			if strings.HasSuffix(a.Filename, ".caf") {
				wanted = []string{"audio"}
				peaks, ok := a.Media["waveform"].([]any)
				if !ok || len(peaks) != 64 {
					return fmt.Errorf("audio waveform missing")
				}
				for _, value := range peaks {
					peak, ok := value.(float64)
					if !ok || peak < 0 || peak > 255 {
						return fmt.Errorf("invalid waveform peak")
					}
				}
			}
			for _, name := range wanted {
				if !contracts.ValidDigest(a.Derivatives[name].Digest) {
					return fmt.Errorf("%s variant missing", name)
				}
			}
			for name, v := range a.Derivatives {
				var content []byte
				lastStatus := 0
				if err := w.Await(90*time.Second, func() bool {
					req, err := http.NewRequestWithContext(ctx, "GET", endpoint.URL+"/v1/blobs/"+v.Digest+"?record="+record.ID, nil)
					if err != nil {
						return false
					}
					req.Host = endpoint.Host
					req.Header.Set("Authorization", "Bearer "+endpoint.Token)
					response, err := world.HTTP.Do(req)
					if err != nil {
						return false
					}
					defer response.Body.Close()
					lastStatus = response.StatusCode
					if response.StatusCode != 200 {
						io.Copy(io.Discard, response.Body)
						return false
					}
					content, err = io.ReadAll(io.LimitReader(response.Body, 32<<20))
					return err == nil && fmt.Sprintf("%x", sha256.Sum256(content)) == v.Digest
				}); err != nil {
					entries := []map[string]any{}
					for _, target := range []world.Endpoint{w.Gateway, endpoint} {
						var status struct {
							Replica struct {
								Blobs contracts.BlobProgress `json:"blobs"`
							} `json:"replica"`
						}
						(scripted.Client{Endpoint: target}).Call(ctx, "sources.status", struct{}{}, &status)
						entries = append(entries, map[string]any{"remote": target.Remote, "entry": status.Replica.Blobs.Entries[v.Digest]})
					}
					json.NewEncoder(os.Stdout).Encode(map[string]any{"status": "failed_readonly", "variant": name, "remote": endpoint.Remote, "http_status": lastStatus, "entries": entries, "verified_variants": output, "new_messages_sent": 0})
					return fmt.Errorf("%s blob unavailable or corrupt", name)
				}
				extension := ".m4a"
				switch v.MIME {
				case "image/jpeg":
					extension = ".jpg"
				case "image/png":
					extension = ".png"
				case "video/mp4":
					extension = ".mp4"
				}
				path := filepath.Join(w.Dir, "verify-"+v.Digest+extension)
				if err := os.WriteFile(path, content, 0600); err != nil {
					return err
				}
				truth, err := w.Truth("observe", map[string]any{"source": "media", "file": path})
				os.Remove(path)
				if err != nil {
					return fmt.Errorf("%s truth unavailable", name)
				}
				checked := false
				if strings.HasPrefix(v.MIME, "image/") {
					maximum := 1600
					if name == "thumb" || name == "poster" {
						maximum = 400
					}
					checked = (truth["type"] == "public.jpeg" || truth["type"] == "public.png") && int(numeric(truth["width"])) == v.Width && int(numeric(truth["height"])) == v.Height && max(v.Width, v.Height) <= maximum
				}
				if v.MIME == "video/mp4" {
					codecs, _ := truth["video"].([]any)
					moov, mdat := bytes.Index(content, []byte("moov")), bytes.Index(content, []byte("mdat"))
					checked = len(codecs) > 0 && codecs[0] == "avc1" && min(v.Width, v.Height) <= 1080 && max(v.Width, v.Height) <= 1920 && moov >= 0 && mdat > moov
				}
				if v.MIME == "audio/mp4" {
					codecs, _ := truth["audio"].([]any)
					checked = len(codecs) > 0 && codecs[0] == "aac "
				}
				if v.MIME == "video/mp4" || v.MIME == "audio/mp4" {
					duration := numeric(truth["duration_ms"])
					checked = checked && duration >= 950 && duration <= 1100
				}
				if !checked {
					json.NewEncoder(os.Stdout).Encode(map[string]any{"status": "failed_readonly", "variant": name, "remote": endpoint.Remote, "truth": truth, "width": v.Width, "height": v.Height, "new_messages_sent": 0, "verified_variants": output})
					return fmt.Errorf("%s codec or dimensions mismatch", name)
				}
				output = append(output, map[string]any{"variant": name, "remote": endpoint.Remote, "mime": v.MIME, "bytes": len(content), "digest_verified": true, "truth": truth})
			}
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"status": "verified_readonly", "new_messages_sent": 0, "latency_budget": "unverified; existing attachments only", "network_path": w.NetworkPath, "variants": output})
}
func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func numeric(value any) float64 {
	switch n := value.(type) {
	case float64:
		return n
	case json.Number:
		v, err := n.Float64()
		if err == nil {
			return v
		}
	}
	return -1
}
