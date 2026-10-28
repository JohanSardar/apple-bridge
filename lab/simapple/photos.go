package simapple

import (
	"encoding/json"
	"slices"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) photosQuery(frame contracts.IPCFrame) (contracts.IPCFrame, bool) {
	if frame.Op != "photos.search" && frame.Op != "photos.original" && frame.Op != "photos.thumbnail" {
		return contracts.IPCFrame{}, false
	}
	out := contracts.IPCFrame{T: "QueryError", ID: frame.ID, Code: "unsupported"}
	if frame.Op != "photos.search" {
		return out, true
	}
	var args struct {
		From     string `json:"from"`
		To       string `json:"to"`
		Album    string `json:"album"`
		Kind     string `json:"kind"`
		Favorite *bool  `json:"favorite"`
		Text     string `json:"text"`
		Limit    *int   `json:"limit"`
	}
	if contracts.DecodeInput(frame.Args, &args) != nil {
		out.Code = "invalid"
		return out, true
	}
	if args.Text != "" {
		return out, true
	}
	limit := 100
	if args.Limit != nil {
		limit = *args.Limit
	}
	if limit < 1 || limit > 1000 {
		out.Code = "invalid"
		return out, true
	}
	from, to := time.Time{}, time.Time{}
	for _, bound := range []struct {
		input  string
		target *time.Time
	}{{args.From, &from}, {args.To, &to}} {
		input, target := bound.input, bound.target
		if input == "" {
			continue
		}
		value, err := time.Parse(time.RFC3339Nano, input)
		if err != nil {
			out.Code = "invalid"
			return out, true
		}
		*target = value
	}
	items := []json.RawMessage{}
	f.mu.Lock()
	defer f.mu.Unlock()
	for _, row := range f.records {
		if row.Collection != "photos.asset" {
			continue
		}
		var data struct {
			Created  string   `json:"created"`
			Kind     string   `json:"kind"`
			Favorite bool     `json:"favorite"`
			Albums   []string `json:"albums"`
		}
		if json.Unmarshal(row.Data, &data) != nil {
			continue
		}
		if args.Kind != "" && args.Kind != data.Kind || args.Favorite != nil && *args.Favorite != data.Favorite || args.Album != "" && !slices.Contains(data.Albums, args.Album) {
			continue
		}
		created, err := time.Parse(time.RFC3339Nano, data.Created)
		if (!from.IsZero() || !to.IsZero()) && err != nil || !from.IsZero() && created.Before(from) || !to.IsZero() && created.After(to) {
			continue
		}
		items = append(items, row.Data)
	}
	truncated := len(items) > limit
	if truncated {
		items = items[:limit]
	}
	out.T = "QueryResult"
	out.Code = ""
	out.ResultJSON, _ = json.Marshal(map[string]any{"items": items, "truncated": truncated})
	out.Blobs = []contracts.QueryBlob{}
	return out, true
}
