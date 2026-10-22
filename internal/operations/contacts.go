package operations

import (
	"context"
	"encoding/json"
	"sort"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) lookupContacts(ctx context.Context, p trust.Principal, q contracts.ContactLookup) (contracts.ReadResult, error) {
	query := strings.TrimSpace(q.Query)
	if q.Limit < 0 || utf8.RuneCountInString(query) > 64 || q.Query != "" && query == "" {
		return contracts.ReadResult{}, contracts.ErrInvalid
	}
	queryLower := strings.ToLower(query)
	phoneQuery := contracts.NormalizeContactHandle(query)
	for _, r := range query {
		if !unicode.IsDigit(r) && !unicode.IsSpace(r) && !strings.ContainsRune("+().-", r) {
			phoneQuery = ""
			break
		}
	}
	limit := q.Limit
	if query != "" && limit == 0 || limit > 25 {
		limit = 25
	}
	result, err := s.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "contacts.contact", Limit: 1000})
	for err == nil && result.Next != "" {
		page, pageErr := s.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "contacts.contact", Limit: 1000, After: result.Next})
		err = pageErr
		result.Records = append(result.Records, page.Records...)
		result.Next = page.Next
		result.Cursor = page.Cursor
	}
	if err != nil {
		return result, err
	}
	filtered := []contracts.Record{}
	names := map[string]string{}
	for _, record := range result.Records {
		var data map[string]any
		if json.Unmarshal(record.Data, &data) != nil {
			return contracts.ReadResult{}, contracts.ErrCorrupt
		}
		match := q.Name == "" && q.Handle == ""
		if q.Name != "" {
			parts := []string{}
			for _, field := range []string{"given_name", "middle_name", "family_name", "nickname", "organization"} {
				value, _ := data[field].(string)
				parts = append(parts, value)
			}
			match = strings.Contains(strings.ToLower(strings.Join(parts, " ")), strings.ToLower(q.Name))
		}
		if q.Handle != "" {
			match = false
			for _, handle := range contracts.ContactHandles(data) {
				if handle == contracts.NormalizeContactHandle(q.Handle) {
					match = true
				}
			}
		}
		parts := []string{}
		for _, field := range []string{"given_name", "middle_name", "family_name", "nickname", "organization"} {
			value, _ := data[field].(string)
			parts = append(parts, value)
		}
		names[record.ID] = strings.ToLower(strings.TrimSpace(strings.Join(parts, " ")))
		if query != "" {
			found := strings.Contains(names[record.ID], queryLower)
			for _, field := range []string{"phones", "emails"} {
				values, _ := data[field].([]any)
				for _, raw := range values {
					entry, _ := raw.(map[string]any)
					value, _ := entry["value"].(string)
					if field == "emails" {
						found = found || strings.Contains(strings.ToLower(value), queryLower)
					} else {
						found = found || phoneQuery != "" && strings.Contains(contracts.NormalizeContactHandle(value), phoneQuery)
					}
				}
			}
			match = match && found
		}
		if match {
			filtered = append(filtered, record)
		}
	}
	if query != "" || limit > 0 {
		sort.Slice(filtered, func(i, j int) bool {
			if names[filtered[i].ID] != names[filtered[j].ID] {
				return names[filtered[i].ID] < names[filtered[j].ID]
			}
			return filtered[i].ID < filtered[j].ID
		})
		if limit > 0 && len(filtered) > limit {
			filtered = filtered[:limit]
		}
	}
	result.Records = filtered
	return result, nil
}
