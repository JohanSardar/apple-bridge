package operations

import (
	"context"
	"encoding/json"
	"strings"
	"unicode"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

func normalizedHandle(value string) string {
	value = strings.ToLower(strings.TrimSpace(value))
	if strings.Contains(value, "@") {
		return value
	}
	var digits strings.Builder
	for _, c := range value {
		if unicode.IsDigit(c) {
			digits.WriteRune(c)
		}
	}
	result := digits.String()
	if len(result) == 10 {
		result = "1" + result
	}
	return result
}

func contactHandles(data map[string]any) []string {
	result := []string{}
	for _, field := range []string{"phones", "emails"} {
		values, _ := data[field].([]any)
		for _, raw := range values {
			value, _ := raw.(map[string]any)
			text, _ := value["value"].(string)
			if key := normalizedHandle(text); key != "" {
				result = append(result, key)
			}
		}
	}
	return result
}

func (s *Service) lookupContacts(ctx context.Context, p trust.Principal, q contracts.ContactLookup) (contracts.ReadResult, error) {
	result, err := s.State.Read(ctx, p.Visibility(), contracts.ReadQuery{Collection: "contacts.contact", Limit: 10000})
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
			for _, handle := range contactHandles(data) {
				if handle == normalizedHandle(q.Handle) {
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

func (s *Service) linkHandles(ctx context.Context, p trust.Principal, result *contracts.ReadResult) error {
	if !p.Visibility()[contracts.Contacts] {
		return nil
	}
	needed := false
	for _, r := range result.Records {
		if r.Collection == "messages.handle" {
			needed = true
		}
	}
	if !needed {
		return nil
	}
	contacts, err := s.lookupContacts(ctx, p, contracts.ContactLookup{})
	if err != nil {
		return err
	}
	matches := map[string]string{}
	for _, r := range contacts.Records {
		var data map[string]any
		json.Unmarshal(r.Data, &data)
		for _, key := range contactHandles(data) {
			if previous, ok := matches[key]; ok && previous != r.ID {
				matches[key] = ""
			} else if !ok {
				matches[key] = r.ID
			}
		}
	}
	for i, r := range result.Records {
		if r.Collection != "messages.handle" {
			continue
		}
		var data map[string]any
		json.Unmarshal(r.Data, &data)
		delete(data, "contact_id")
		address, _ := data["address"].(string)
		if id := matches[normalizedHandle(address)]; id != "" {
			data["contact_id"] = id
		}
		result.Records[i].Data, _ = json.Marshal(data)
	}
	return nil
}
