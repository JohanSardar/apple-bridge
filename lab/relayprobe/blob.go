package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"time"
)

func blob(ctx context.Context) error {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	var in struct {
		input
		Title  string
		Digest string
		Size   int64
	}
	if err := json.NewDecoder(os.Stdin).Decode(&in); err != nil {
		return err
	}
	ticker := time.NewTicker(20 * time.Millisecond)
	defer ticker.Stop()
	for {
		records, err := named(ctx, in.input, "system.source", in.Title)
		if err != nil {
			return err
		}
		if len(records) == 1 {
			req, err := http.NewRequestWithContext(ctx, "GET", in.URL+"/v1/blobs/"+in.Digest+"?record="+records[0].ID, nil)
			if err != nil {
				return err
			}
			req.Host = in.Host
			req.Header.Set("Authorization", "Bearer "+in.Token)
			response, err := http.DefaultClient.Do(req)
			if err != nil {
				return err
			}
			raw, readErr := io.ReadAll(io.LimitReader(response.Body, in.Size+1))
			response.Body.Close()
			if response.StatusCode == 200 {
				digest := sha256.Sum256(raw)
				if readErr != nil || int64(len(raw)) != in.Size || hex.EncodeToString(digest[:]) != in.Digest {
					return fmt.Errorf("blob bytes differ")
				}
				return json.NewEncoder(os.Stdout).Encode(map[string]any{"bytes": len(raw), "digest_matches": true})
			}
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}
