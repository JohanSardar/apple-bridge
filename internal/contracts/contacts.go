package contracts

import (
	"bytes"
	"encoding/base64"
	"encoding/json"
	"strings"
	"unicode"
)

type ContactLookup struct {
	Query  string `json:"q,omitempty" description:"Partial name, normalized phone or email; 1–64 characters after trimming"`
	Limit  int    `json:"limit,omitempty" description:"New q searches default to 25; positive limits are capped at 25"`
	Name   string `json:"name,omitempty"`
	Handle string `json:"handle,omitempty"`
}

var ContactFields = []string{"previous_family_name", "phonetic_organization", "given_name", "middle_name", "family_name", "nickname", "name_prefix", "name_suffix", "phonetic_given_name", "phonetic_middle_name", "phonetic_family_name", "organization", "job_title", "department", "phones", "emails", "postal_addresses", "urls", "birthday", "non_gregorian_birthday", "dates", "social_profiles", "instant_messages", "relations", "image"}

func validateContactCommand(c Command) (ReminderArgs, error) {
	if ContactGroupOperation(c.Op) {
		return validateContactGroupCommand(c)
	}
	a := ReminderArgs{Contact: c.Args}
	if c.ID == "" || len(c.ID) > 200 || c.WaitMS < 0 || c.WaitMS > 15000 {
		return a, ErrInvalid
	}
	var fields map[string]json.RawMessage
	if DecodeInput(c.Args, &fields) != nil {
		return a, ErrInvalid
	}
	if value, ok := fields["id"]; ok {
		if json.Unmarshal(value, &a.ID) != nil || a.ID == "" {
			return a, ErrInvalid
		}
	}
	for k, v := range fields {
		if k == "id" {
			continue
		}
		found := false
		for _, field := range ContactFields {
			if field == k {
				found = true
			}
		}
		if !found || !validContactField(k, v) {
			return a, ErrInvalid
		}
		if strings.HasSuffix(k, "name") || k == "name_prefix" || k == "name_suffix" || k == "organization" || k == "job_title" || k == "department" {
			var value string
			if json.Unmarshal(v, &value) != nil {
				return a, ErrInvalid
			}
		}
	}
	switch c.Op {
	case "contacts.create":
		if a.ID != "" || c.ExpectedVersion != 0 || len(fields) == 0 {
			return a, ErrInvalid
		}
	case "contacts.update":
		if a.ID == "" || c.ExpectedVersion == 0 || len(fields) < 2 {
			return a, ErrInvalid
		}
	case "contacts.delete":
		if a.ID == "" || c.ExpectedVersion == 0 || len(fields) != 1 {
			return a, ErrInvalid
		}
	default:
		return a, ErrUnsupported
	}
	return a, nil
}

func validContactField(field string, raw json.RawMessage) bool {
	if field == "image" {
		if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
			return true
		}
		var value struct {
			Base64 string `json:"base64"`
		}
		if DecodeInput(raw, &value) != nil {
			return false
		}
		decoded, err := base64.StdEncoding.DecodeString(value.Base64)
		return err == nil && len(decoded) > 0 && len(decoded) <= 8<<20
	}
	if field == "birthday" || field == "non_gregorian_birthday" {
		return validContactDate(raw)
	}
	switch field {
	case "phones", "emails", "urls", "relations", "dates", "postal_addresses", "social_profiles", "instant_messages":
		var entries []map[string]json.RawMessage
		if len(raw) == 0 || raw[0] != '[' || json.Unmarshal(raw, &entries) != nil {
			return false
		}
		for _, entry := range entries {
			for k := range entry {
				if k != "label" && k != "value" {
					return false
				}
			}
			if label, ok := entry["label"]; ok && string(label) != "null" {
				var value string
				if json.Unmarshal(label, &value) != nil {
					return false
				}
			}
			value, ok := entry["value"]
			if !ok {
				return false
			}
			switch field {
			case "phones", "emails", "urls", "relations":
				var text string
				if json.Unmarshal(value, &text) != nil || string(value) == "null" {
					return false
				}
			case "dates":
				if !validContactDate(value) || string(value) == "null" {
					return false
				}
			default:
				var object map[string]string
				if DecodeInput(value, &object) != nil || object == nil {
					return false
				}
			}
		}
		return true
	}
	var value string
	return json.Unmarshal(raw, &value) == nil && string(raw) != "null"
}

func validContactDate(raw json.RawMessage) bool {
	if bytes.Equal(bytes.TrimSpace(raw), []byte("null")) {
		return true
	}
	var value struct {
		Year        *int   `json:"year"`
		IsLeapMonth *bool  `json:"is_leap_month"`
		Month       int    `json:"month"`
		Day         int    `json:"day"`
		Calendar    string `json:"calendar"`
	}
	return DecodeInput(raw, &value) == nil && value.Month >= 1 && value.Month <= 12 && value.Day >= 1 && value.Day <= 31
}

func NormalizeContactHandle(value string) string {
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

func ContactHandles(data map[string]any) []string {
	result := []string{}
	for _, field := range []string{"phones", "emails"} {
		values, _ := data[field].([]any)
		for _, raw := range values {
			value, _ := raw.(map[string]any)
			text, _ := value["value"].(string)
			if key := NormalizeContactHandle(text); key != "" {
				result = append(result, key)
			}
		}
	}
	return result
}

func ContactGroupOperation(op string) bool {
	switch op {
	case "contacts.create_group", "contacts.rename_group", "contacts.delete_group", "contacts.add_member", "contacts.remove_member":
		return true
	}
	return false
}

type ContactGroupArgs struct {
	ID        string `json:"id,omitempty"`
	Name      string `json:"name,omitempty"`
	ContactID string `json:"contact_id,omitempty"`
}

func validateContactGroupCommand(c Command) (ReminderArgs, error) {
	a := ReminderArgs{Contact: c.Args}
	var group ContactGroupArgs
	if c.ID == "" || len(c.ID) > 200 || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &group) != nil {
		return a, ErrInvalid
	}
	a.ID = group.ID
	named := strings.TrimSpace(group.Name) != "" && len(group.Name) <= 256
	switch c.Op {
	case "contacts.create_group":
		if group.ID != "" || c.ExpectedVersion != 0 || !named || group.ContactID != "" {
			return a, ErrInvalid
		}
	case "contacts.rename_group":
		if group.ID == "" || c.ExpectedVersion == 0 || !named || group.ContactID != "" {
			return a, ErrInvalid
		}
	case "contacts.delete_group":
		if group.ID == "" || c.ExpectedVersion == 0 || group.Name != "" || group.ContactID != "" {
			return a, ErrInvalid
		}
	case "contacts.add_member", "contacts.remove_member":
		if group.ID == "" || c.ExpectedVersion == 0 || group.Name != "" || group.ContactID == "" {
			return a, ErrInvalid
		}
	default:
		return a, ErrUnsupported
	}
	return a, nil
}
