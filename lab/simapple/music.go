package simapple

import (
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"strings"
)

func simulateMusicQuery(op string, raw json.RawMessage) (string, json.RawMessage, bool) {
	if op != "music.search_catalog" {
		return "", nil, false
	}
	var fields map[string]json.RawMessage
	if json.Unmarshal(raw, &fields) != nil {
		return "invalid", nil, true
	}
	for _, key := range []string{"term", "entity", "limit"} {
		if string(fields[key]) == "null" {
			return "invalid", nil, true
		}
	}
	var args struct {
		Term   string `json:"term"`
		Entity string `json:"entity"`
		Limit  *int   `json:"limit"`
	}
	if contracts.DecodeInput(raw, &args) != nil || strings.TrimSpace(args.Term) == "" || len(args.Term) > 512 || (args.Limit != nil && (*args.Limit < 1 || *args.Limit > 100)) || (args.Entity != "" && args.Entity != "song" && args.Entity != "album" && args.Entity != "musicArtist") {
		return "invalid", nil, true
	}
	entity := args.Entity
	if entity == "" {
		entity = "song"
	}
	result, _ := json.Marshal(map[string]any{"items": []any{map[string]string{"catalog_id": "1441164738", "name": "Fixture", "artist": "Fixture artist", "kind": entity}}})
	return "", result, true
}
