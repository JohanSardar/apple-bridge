package state

import (
	"cmp"
	"context"
	"database/sql"
	"slices"
	"strings"
	"unicode"
	"unicode/utf8"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"golang.org/x/text/unicode/norm"
)

func (s *Store) ContactsList(ctx context.Context, visibility trust.Visibility, query contracts.ContactsListQuery) (contracts.ContactsListResult, error) {
	out := contracts.ContactsListResult{Sections: []contracts.ContactListSection{}, Groups: []contracts.ContactListGroup{}, Containers: []contracts.Record{}, Sort: query.Sort, DisplayOrder: query.DisplayOrder}
	if out.Sort == "" {
		out.Sort = "family"
	}
	if out.DisplayOrder == "" {
		out.DisplayOrder = "given"
	}
	if !slices.Contains([]string{"given", "family"}, out.Sort) || !slices.Contains([]string{"given", "family"}, out.DisplayOrder) {
		return out, contracts.ErrInvalid
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return out, err
	}
	defer tx.Rollback()
	out.Cursor, err = cursorAt(ctx, tx)
	if err != nil || !visibility[contracts.Contacts] {
		return out, err
	}
	records, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection IN ('contacts.contact','contacts.group','contacts.container') AND deleted=0 ORDER BY collection,id")
	if err != nil {
		return out, err
	}
	contacts := map[string]contracts.Record{}
	groups := []contracts.Record{}
	for _, record := range records {
		switch record.Collection {
		case "contacts.contact":
			contacts[record.ID] = record
		case "contacts.group":
			groups = append(groups, record)
		case "contacts.container":
			out.Containers = append(out.Containers, record)
		}
	}
	var members map[string]bool
	for _, group := range groups {
		ids, _ := modelFields(group)["member_ids"].([]any)
		unique := map[string]bool{}
		for _, raw := range ids {
			if id, ok := raw.(string); ok {
				if _, exists := contacts[id]; exists {
					unique[id] = true
				}
			}
		}
		out.Groups = append(out.Groups, contracts.ContactListGroup{Group: group, Count: len(unique)})
		if group.ID == query.Group {
			members = unique
		}
	}
	if query.Group != "" && members == nil {
		return out, contracts.ErrNotFound
	}
	slices.SortFunc(out.Groups, func(a, b contracts.ContactListGroup) int {
		return cmp.Or(cmp.Compare(contactFold(fieldString(modelFields(a.Group), "name")), contactFold(fieldString(modelFields(b.Group), "name"))), cmp.Compare(a.Group.ID, b.Group.ID))
	})
	conversations := map[string][]string{}
	if visibility[contracts.Messages] {
		rows, err := tx.QueryContext(ctx, `SELECT DISTINCT json_extract(h.payload,'$.data.contact_id'),p.chat_id FROM records h JOIN chat_participant p ON p.handle_id=h.id JOIN records c ON c.collection='messages.chat' AND c.id=p.chat_id AND c.deleted=0 WHERE h.collection='messages.handle' AND h.deleted=0 AND json_extract(h.payload,'$.data.contact_id') IS NOT NULL ORDER BY p.chat_id`)
		if err != nil {
			return out, err
		}
		for rows.Next() {
			var contact, chat string
			if err = rows.Scan(&contact, &chat); err != nil {
				rows.Close()
				return out, err
			}
			conversations[contact] = append(conversations[contact], chat)
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return out, err
		}
	}
	items := []contracts.ContactListItem{}
	for id, record := range contacts {
		if members != nil && !members[id] {
			continue
		}
		fields := modelFields(record)
		item := contracts.ContactListItem{Contact: record, DisplayName: contactName(fields, out.DisplayOrder), SortKey: contactFold(contactName(fields, out.Sort)), ConversationIDs: []string{}}
		if item.DisplayName == "" {
			item.DisplayName = "No Name"
		}
		if me, ok := fields["is_me"].(bool); ok {
			item.IsMe = &me
		}
		if ids := conversations[id]; len(ids) > 0 {
			item.ConversationIDs = ids
		}
		for _, key := range []string{"thumbnail_image", "image"} {
			if image, ok := fields[key].(map[string]any); ok && contracts.ValidDigest(fieldString(image, "digest")) && fieldString(image, "state") != "unavailable" {
				item.Avatar = &contracts.HandleAvatar{Digest: fieldString(image, "digest"), Variant: "original"}
				break
			}
		}
		for _, part := range strings.Fields(item.DisplayName) {
			first, _ := utf8.DecodeRuneInString(part)
			if utf8.RuneCountInString(item.Initials) < 2 {
				item.Initials += strings.ToUpper(string(first))
			}
		}
		items = append(items, item)
	}
	slices.SortFunc(items, func(a, b contracts.ContactListItem) int {
		return cmp.Or(cmp.Compare(contactSection(a.SortKey), contactSection(b.SortKey)), cmp.Compare(a.SortKey, b.SortKey), cmp.Compare(a.Contact.ID, b.Contact.ID))
	})
	for _, item := range items {
		key := contactSection(item.SortKey)
		if len(out.Sections) == 0 || out.Sections[len(out.Sections)-1].Key != key {
			out.Sections = append(out.Sections, contracts.ContactListSection{Key: key, Contacts: []contracts.ContactListItem{}})
		}
		i := len(out.Sections) - 1
		out.Sections[i].Contacts = append(out.Sections[i].Contacts, item)
	}
	out.Count = len(items)
	return out, nil
}
func contactName(fields map[string]any, order string) string {
	if fieldString(fields, "contact_type") == "organization" && strings.TrimSpace(fieldString(fields, "organization")) != "" {
		return strings.TrimSpace(fieldString(fields, "organization"))
	}
	keys := []string{"given_name", "middle_name", "family_name"}
	if order == "family" {
		keys = []string{"family_name", "given_name", "middle_name"}
	}
	parts := []string{}
	for _, key := range keys {
		if value := strings.TrimSpace(fieldString(fields, key)); value != "" {
			parts = append(parts, value)
		}
	}
	if len(parts) > 0 {
		return strings.Join(parts, " ")
	}
	for _, key := range []string{"nickname", "organization"} {
		if value := strings.TrimSpace(fieldString(fields, key)); value != "" {
			return value
		}
	}
	return ""
}
func contactFold(value string) string {
	return strings.ToLower(strings.Map(func(r rune) rune {
		if unicode.Is(unicode.Mn, r) {
			return -1
		}
		return r
	}, norm.NFD.String(value)))
}
func contactSection(value string) string {
	first, _ := utf8.DecodeRuneInString(value)
	if first >= 'a' && first <= 'z' {
		return strings.ToUpper(string(first))
	}
	return "#"
}
