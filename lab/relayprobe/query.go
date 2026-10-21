package main

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type input struct {
	URL        string
	Host       string
	Token      string
	Home       string
	Collection string
	ProxyReady string
}

func request(ctx context.Context, in input, path string, value, output any) error {
	raw, err := json.Marshal(value)
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, "POST", in.URL+"/v1/op/"+path, bytes.NewReader(raw))
	if err != nil {
		return err
	}
	req.Host = in.Host
	req.Header.Set("Authorization", "Bearer "+in.Token)
	req.Header.Set("Content-Type", "application/json")
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	if resp.StatusCode != 200 {
		return fmt.Errorf("local API status %d", resp.StatusCode)
	}
	return json.NewDecoder(io.LimitReader(resp.Body, 4<<20)).Decode(output)
}
func query(ctx context.Context) error {
	var in input
	if err := json.NewDecoder(os.Stdin).Decode(&in); err != nil {
		return err
	}
	records := []contracts.Record{}
	var cursor contracts.Cursor
	after := ""
	for {
		var page contracts.ReadResult
		if err := request(ctx, in, "records.query", contracts.ReadQuery{Collection: in.Collection, After: after, Limit: 200}, &page); err != nil {
			return err
		}
		records = append(records, page.Records...)
		cursor = page.Cursor
		if page.Next == "" {
			break
		}
		after = page.Next
	}
	raw, _ := json.Marshal(records)
	digest := sha256.Sum256(raw)
	var status struct {
		Replica contracts.ReplicaStatus `json:"replica"`
	}
	if err := request(ctx, in, "sources.status", map[string]any{}, &status); err != nil {
		return err
	}
	var credentials struct {
		Enrollment *trust.Enrollment `json:"enrollment"`
	}
	raw, err := os.ReadFile(in.Home + "/credentials.json")
	if err != nil {
		return err
	}
	if err = json.Unmarshal(raw, &credentials); err != nil {
		return err
	}
	var route struct {
		Connections int `json:"connections"`
	}
	raw, err = os.ReadFile(in.ProxyReady)
	if err != nil {
		return err
	}
	if err = json.Unmarshal(raw, &route); err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"digest": hex.EncodeToString(digest[:]), "count": len(records), "cursor": cursor, "status": status.Replica, "enrollment": credentials.Enrollment, "public_connections": route.Connections})
}
