package contracts

import (
	"encoding/json"
	"strings"
)

type ContactLookup struct {
	Name   string `json:"name,omitempty"`
	Handle string `json:"handle,omitempty"`
}

var ContactFields = []string{"previous_family_name", "phonetic_organization", "given_name", "middle_name", "family_name", "nickname", "name_prefix", "name_suffix", "phonetic_given_name", "phonetic_middle_name", "phonetic_family_name", "organization", "job_title", "department", "phones", "emails", "postal_addresses", "urls", "birthday", "non_gregorian_birthday", "dates", "social_profiles", "instant_messages", "relations", "image"}

func validateContactCommand(c Command) (ReminderArgs, error) {
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
		if !found {
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
