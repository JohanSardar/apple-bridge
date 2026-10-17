package operations

import (
	"context"
	"encoding/json"
	"strings"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func (s *Service) lookupContacts(ctx context.Context, p trust.Principal, q contracts.ContactLookup) (contracts.ReadResult, error) {
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
		if match {
			filtered = append(filtered, record)
		}
	}
	result.Records = filtered
	return result, nil
}
