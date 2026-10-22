package state

import (
	"context"
	"crypto/sha256"
	"database/sql"
	"encoding/hex"
	"encoding/json"
	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
	"slices"
	"strings"
	"unicode"
	"unicode/utf8"
)

func (s *Store) SearchMessages(ctx context.Context, v trust.Visibility, q contracts.MessageSearchQuery) (contracts.MessageSearchResult, error) {
	out := contracts.MessageSearchResult{Items: []contracts.ThreadItem{}, CoverageNotes: []string{}}
	q.Text = strings.TrimSpace(q.Text)
	limit, err := viewLimit(q.Limit)
	if err != nil || !utf8.ValidString(q.Text) || utf8.RuneCountInString(q.Text) > 512 || len(q.FromHandleID) > 128 || len(q.FromContactID) > 128 || q.FromHandleID != "" && q.FromContactID != "" || !slices.Contains([]string{"", "photos", "links", "documents", "locations"}, q.Kind) || q.Text == "" && q.Kind == "" && q.FromHandleID == "" && q.FromContactID == "" && !q.Unread {
		return out, contracts.ErrInvalid
	}
	if q.FromContactID != "" && !v[contracts.Contacts] {
		return out, contracts.ErrUnauthorized
	}
	identity := q
	identity.After = ""
	identity.Limit = 0
	raw, _ := json.Marshal(identity)
	hash := sha256.Sum256(raw)
	scope := hex.EncodeToString(hash[:])
	var edge viewCursor
	if q.After != "" {
		edge, err = decodeView(q.After, "message-search", scope)
		if err != nil {
			return out, err
		}
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
	where := "c.deleted=0 AND coalesce(json_extract(c.payload,'$.data.filter'),'')!='spam' AND coalesce(json_extract(c.payload,'$.data.is_spam'),0)=0 AND coalesce(json_extract(c.payload,'$.data.is_archived'),0)=0 AND json_extract(c.payload,'$.data.deleted_at') IS NULL AND json_extract(m.payload,'$.data.deleted_at') IS NULL AND json_extract(m.payload,'$.data.retracted_at') IS NULL AND coalesce(json_extract(m.payload,'$.data.is_spam'),0)=0"
	args := []any{}
	if q.Text != "" {
		if err = requireSearchReady(ctx, tx); err != nil {
			return out, err
		}
		terms := strings.FieldsFunc(q.Text, func(value rune) bool {
			return !unicode.IsLetter(value) && !unicode.IsNumber(value) && !unicode.IsMark(value)
		})
		if len(terms) == 0 {
			return out, nil
		}
		for i, term := range terms {
			terms[i] = "\"" + strings.ReplaceAll(term, "\"", "\"\"") + "\"*"
		}
		match := strings.Join(terms, " AND ")
		where += " AND (m.id IN (SELECT id FROM records_fts WHERE collection='messages.message' AND records_fts MATCH ?) OR m.id IN (SELECT a.message_id FROM attachment_by_message a JOIN records_fts ON records_fts.collection='messages.attachment' AND records_fts.id=a.id WHERE records_fts MATCH ? AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0))"
		args = append(args, match, match)
	}
	if q.FromHandleID != "" {
		where += " AND json_extract(m.payload,'$.data.sender_handle_id')=?"
		args = append(args, q.FromHandleID)
	}
	if q.FromContactID != "" {
		where += " AND json_extract(m.payload,'$.data.sender_handle_id') IN (SELECT id FROM records WHERE collection='messages.handle' AND deleted=0 AND json_extract(payload,'$.data.contact_id')=?)"
		args = append(args, q.FromContactID)
	}
	if q.Unread {
		where += " AND m.unread=1"
	}
	switch q.Kind {
	case "photos":
		where += " AND EXISTS(SELECT 1 FROM attachment_by_message a WHERE a.message_id=m.id AND a.kind IN ('photos','videos') AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0)"
	case "documents":
		where += " AND EXISTS(SELECT 1 FROM attachment_by_message a WHERE a.message_id=m.id AND a.kind='documents' AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0)"
	case "links":
		where += " AND json_extract(m.payload,'$.data.balloon.kind')='link'"
	case "locations":
		where += " AND json_extract(m.payload,'$.data.balloon.kind')='find_my'"
	}
	if q.After != "" {
		where += " AND (m.sent_at<? OR (m.sent_at=? AND m.id>?))"
		args = append(args, edge.At, edge.At, edge.ID)
	}
	args = append(args, limit+1)
	records, err := readModelRecords(ctx, tx, "SELECT m.payload FROM msg_by_chat m JOIN records c ON c.collection='messages.chat' AND c.id=m.chat_id WHERE "+where+" ORDER BY m.sent_at DESC,m.id LIMIT ?", args...)
	if err != nil {
		return out, err
	}
	if len(records) > limit {
		records = records[:limit]
		last := records[len(records)-1]
		out.Next = encodeView("message-search", scope, modelTime(fieldString(modelFields(last), "sent_at")), last.ID)
	}
	if len(records) == 0 {
		return out, nil
	}
	ids := make([]any, len(records))
	for index, record := range records {
		ids[index] = record.ID
	}
	slots := strings.TrimSuffix(strings.Repeat("?,", len(ids)), ",")
	attachments, err := readModelRecords(ctx, tx, "SELECT payload FROM attachment_by_message WHERE message_id IN ("+slots+") AND coalesce(json_extract(payload,'$.data.hidden'),0)=0 ORDER BY id", ids...)
	if err != nil {
		return out, err
	}
	byMessage := map[string][]contracts.Record{}
	for _, attachment := range attachments {
		id := attachmentMessage(modelFields(attachment))
		byMessage[id] = append(byMessage[id], attachment)
	}
	for _, record := range records {
		items := byMessage[record.ID]
		if items == nil {
			items = []contracts.Record{}
		}
		out.Items = append(out.Items, contracts.ThreadItem{Record: record, Attachments: items, Reactions: []contracts.Record{}})
	}
	return out, nil
}
