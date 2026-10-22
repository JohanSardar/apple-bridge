package web

import (
	"encoding/json"
	"io"
	"net/http"
	"strings"
	"unicode/utf8"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
)

type recipientHandle struct {
	Value string `json:"value"`
	Kind  string `json:"kind"`
	Label string `json:"label"`
}

type recipientContact struct {
	ContactID   string            `json:"contact_id"`
	DisplayName string            `json:"display_name"`
	Handles     []recipientHandle `json:"handles"`
}

func (s *server) lookup(w http.ResponseWriter, r *http.Request) {
	if !s.allowRate("lookup:"+current(r).Principal.ID, false, 20) {
		w.Header().Set("Retry-After", "1")
		reject(w, 429, "rate_limited")
		return
	}
	var args struct {
		Query string `json:"q"`
		Limit *int   `json:"limit,omitempty"`
	}
	decoder := json.NewDecoder(http.MaxBytesReader(w, r.Body, 4096))
	decoder.DisallowUnknownFields()
	if decoder.Decode(&args) != nil || decoder.Decode(new(any)) != io.EOF {
		reject(w, 400, "invalid")
		return
	}
	args.Query = strings.TrimSpace(args.Query)
	limit := 8
	if args.Limit != nil {
		limit = *args.Limit
	}
	if utf8.RuneCountInString(args.Query) < 1 || utf8.RuneCountInString(args.Query) > 64 || limit < 1 || limit > 20 {
		reject(w, 400, "invalid")
		return
	}
	results := []recipientContact{}
	if s.deps.Authority.Authorize(current(r).Principal, "contacts:read") != nil {
		respond(w, 200, map[string]any{"results": results, "capability": "unavailable"})
		return
	}
	if s.deps.Operations == nil {
		reject(w, 503, "unavailable")
		return
	}
	input, _ := json.Marshal(contracts.ContactLookup{Query: args.Query, Limit: limit})
	value, err := s.deps.Operations.Invoke(r.Context(), current(r).Principal, "contacts.lookup", input)
	if err != nil {
		problem := safeError(err)
		status := map[string]int{"unauthorized": 401, "forbidden": 403, "unavailable": 503}[problem.Code]
		if status == 0 {
			status = 400
		}
		reject(w, status, problem.Code)
		return
	}
	rows, ok := value.(contracts.ReadResult)
	if !ok {
		reject(w, 503, "unavailable")
		return
	}
	for _, record := range rows.Records {
		var data map[string]any
		if json.Unmarshal(record.Data, &data) != nil {
			reject(w, 503, "unavailable")
			return
		}
		parts := []string{}
		for _, field := range []string{"given_name", "middle_name", "family_name"} {
			if value, _ := data[field].(string); strings.TrimSpace(value) != "" {
				parts = append(parts, strings.TrimSpace(value))
			}
		}
		name := strings.Join(parts, " ")
		for _, field := range []string{"nickname", "organization"} {
			if name == "" {
				name, _ = data[field].(string)
			}
		}
		contact := recipientContact{ContactID: record.ID, DisplayName: name, Handles: []recipientHandle{}}
		for _, field := range []string{"phones", "emails"} {
			entries, _ := data[field].([]any)
			for _, raw := range entries {
				entry, _ := raw.(map[string]any)
				value, _ := entry["value"].(string)
				label, _ := entry["label"].(string)
				if strings.TrimSpace(value) == "" {
					continue
				}
				kind := "phone"
				if field == "emails" {
					kind = "email"
				}
				contact.Handles = append(contact.Handles, recipientHandle{Value: value, Kind: kind, Label: label})
			}
		}
		if contact.DisplayName == "" && len(contact.Handles) > 0 {
			contact.DisplayName = contact.Handles[0].Value
		}
		results = append(results, contact)
	}
	respond(w, 200, map[string]any{"results": results, "capability": "available"})
}
