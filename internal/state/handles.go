package state

import (
	"context"
	"database/sql"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"strings"
	"unicode"
	"unicode/utf8"
)

func (s *Store) Handles(ctx context.Context, v trust.Visibility, q contracts.HandlesQuery) (contracts.HandlesResult, error) {
	out := contracts.HandlesResult{Handles: []contracts.HandleSummary{}}
	if len(q.IDs) > 500 {
		return out, contracts.ErrInvalid
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return out, err
	}
	defer tx.Rollback()
	out.Cursor, err = cursorAt(ctx, tx)
	if err != nil || !v[contracts.Messages] {
		return out, err
	}
	seen := map[string]bool{}
	for _, id := range q.IDs {
		if id == "" {
			return out, contracts.ErrInvalid
		}
		if seen[id] {
			continue
		}
		seen[id] = true
		h, err := modelRecord(ctx, tx, "messages.handle", id)
		if err == contracts.ErrNotFound {
			continue
		}
		if err != nil {
			return out, err
		}
		summary, err := handleSummary(ctx, tx, v, h)
		if err != nil {
			return out, err
		}
		out.Handles = append(out.Handles, summary)
	}
	return out, nil
}
func handleSummary(ctx context.Context, tx *sql.Tx, v trust.Visibility, h contracts.Record) (contracts.HandleSummary, error) {
	d := modelFields(h)
	out := contracts.HandleSummary{HandleID: h.ID, Address: fieldString(d, "address"), Service: fieldString(d, "service"), DisplayName: fieldString(d, "address")}
	if cid := fieldString(d, "contact_id"); cid != "" && v[contracts.Contacts] {
		contact, err := modelRecord(ctx, tx, "contacts.contact", cid)
		if err != nil && err != contracts.ErrNotFound {
			return out, err
		}
		if err == nil {
			out.ContactID = &cid
			cd := modelFields(contact)
			out.GivenName = fieldString(cd, "given_name")
			out.FamilyName = fieldString(cd, "family_name")
			parts := []string{}
			for _, key := range []string{"given_name", "middle_name", "family_name"} {
				if p := fieldString(cd, key); p != "" {
					parts = append(parts, p)
				}
			}
			if len(parts) > 0 {
				out.DisplayName = strings.Join(parts, " ")
			} else if n := fieldString(cd, "nickname"); n != "" {
				out.DisplayName = n
			} else if n := fieldString(cd, "organization"); n != "" {
				out.DisplayName = n
			}
			for _, key := range []string{"thumbnail_image", "image"} {
				if image, ok := cd[key].(map[string]any); ok && fieldString(image, "digest") != "" {
					out.Avatar = &contracts.HandleAvatar{Digest: fieldString(image, "digest"), Variant: "original"}
					break
				}
			}
		}
	}
	words := strings.FieldsFunc(out.DisplayName, func(r rune) bool { return unicode.IsSpace(r) || r == '@' })
	if len(words) > 0 {
		a, _ := utf8.DecodeRuneInString(words[0])
		out.Initials = strings.ToUpper(string(a))
		if len(words) > 1 && !strings.Contains(out.DisplayName, "@") {
			b, _ := utf8.DecodeRuneInString(words[len(words)-1])
			out.Initials += strings.ToUpper(string(b))
		}
	}
	return out, nil
}
