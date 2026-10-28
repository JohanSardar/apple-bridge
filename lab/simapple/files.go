package simapple

import (
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

func (f *fakeApple) filesQuery(frame contracts.IPCFrame) (contracts.IPCFrame, bool) {
	if frame.Op != "files.search" && frame.Op != "files.read" {
		return contracts.IPCFrame{}, false
	}
	out := contracts.IPCFrame{T: "QueryResult", ID: frame.ID, Blobs: []contracts.QueryBlob{}}
	var args struct {
		Root  string `json:"root"`
		Path  string `json:"path"`
		Query string `json:"query"`
		Field string `json:"field"`
	}
	if contracts.DecodeInput(frame.Args, &args) != nil {
		out.T = "QueryError"
		out.Code = "invalid"
		return out, true
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	items := []json.RawMessage{}
	for _, row := range f.records {
		if row.Collection != "files.item" {
			continue
		}
		var data struct {
			Root   string `json:"root"`
			Path   string `json:"path"`
			Name   string `json:"name"`
			Digest string `json:"digest"`
		}
		if json.Unmarshal(row.Data, &data) != nil {
			continue
		}
		if frame.Op == "files.search" && strings.Contains(strings.ToLower(data.Name), strings.ToLower(args.Query)) {
			items = append(items, row.Data)
		}
		if frame.Op == "files.read" && data.Root == args.Root && data.Path == args.Path {
			bytes, ok := f.blobs[data.Digest]
			if !ok {
				break
			}
			out.ResultJSON, _ = json.Marshal(map[string]any{"digest": data.Digest, "size": len(bytes)})
			out.Blobs = []contracts.QueryBlob{{Digest: data.Digest, Size: int64(len(bytes)), Handle: data.Digest}}
			return out, true
		}
	}
	if frame.Op == "files.search" {
		out.ResultJSON, _ = json.Marshal(map[string]any{"items": items})
		return out, true
	}
	out.T = "QueryError"
	out.Code = "unavailable"
	return out, true
}
