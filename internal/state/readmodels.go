package state

import (
	"context"
	"database/sql"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type viewCursor struct {
	View  string `json:"v"`
	Scope string `json:"s"`
	At    string `json:"t"`
	ID    string `json:"i"`
}

func encodeView(view, scope, at, id string) string {
	b, _ := json.Marshal(viewCursor{view, scope, at, id})
	return base64.RawURLEncoding.EncodeToString(b)
}
func decodeView(raw, view, scope string) (viewCursor, error) {
	var c viewCursor
	b, err := base64.RawURLEncoding.DecodeString(raw)
	if err != nil || json.Unmarshal(b, &c) != nil || c.View != view || c.Scope != scope || c.ID == "" {
		return c, contracts.ErrInvalid
	}
	return c, nil
}
func viewLimit(n int) (int, error) {
	if n < 0 || n > 200 {
		return 0, contracts.ErrInvalid
	}
	if n == 0 {
		n = 50
	}
	return n, nil
}
func modelTime(raw string) string {
	if t, err := time.Parse(time.RFC3339Nano, raw); err == nil {
		return t.UTC().Format("2006-01-02T15:04:05.000000000Z")
	}
	return raw
}
func readModelRecords(ctx context.Context, tx *sql.Tx, query string, args ...any) ([]contracts.Record, error) {
	rows, err := tx.QueryContext(ctx, query, args...)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	result := []contracts.Record{}
	for rows.Next() {
		var raw []byte
		var r contracts.Record
		if err = rows.Scan(&raw); err != nil {
			return nil, err
		}
		if err = json.Unmarshal(raw, &r); err != nil {
			return nil, err
		}
		result = append(result, r)
	}
	return result, rows.Err()
}
func modelRecord(ctx context.Context, tx *sql.Tx, collection, id string) (contracts.Record, error) {
	r, err := recordAt(ctx, tx, collection, id)
	if err == sql.ErrNoRows || r.Deleted {
		return contracts.Record{}, contracts.ErrNotFound
	}
	return r, err
}
func (s *Store) Chats(ctx context.Context, v trust.Visibility, q contracts.ChatsQuery) (contracts.ChatsResult, error) {
	out := contracts.ChatsResult{Chats: []contracts.ChatSummary{}, CoverageNotes: []string{"reactions without sent_at use observed_at"}}
	limit, err := viewLimit(q.Limit)
	if err != nil {
		return out, err
	}
	if !slices.Contains([]string{"", "all", "known", "unknown", "unread", "deleted", "spam", "archived"}, q.Filter) {
		return out, contracts.ErrInvalid
	}
	var participants []string
	if q.Participants != nil {
		if len(q.Participants) == 0 || len(q.Participants) > 32 {
			return out, contracts.ErrInvalid
		}
		participants = []string{}
		for _, handle := range q.Participants {
			normalized := contracts.NormalizeContactHandle(handle)
			if normalized == "" || utf8.RuneCountInString(handle) > 254 {
				return out, contracts.ErrInvalid
			}
			participants = append(participants, normalized)
		}
		slices.Sort(participants)
		participants = slices.Compact(participants)
	}
	scope := participantScope(q.Filter, participants)
	var c viewCursor
	if q.After != "" {
		c, err = decodeView(q.After, "chats", scope)
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
	where := "r.collection='messages.chat' AND r.deleted=0"
	args := []any{}
	if participants != nil {
		predicate, values := exactParticipantPredicate(participants)
		where += " AND " + predicate
		args = append(args, values...)
	}
	if q.Filter == "archived" {
		where += " AND coalesce(json_extract(r.payload,'$.data.is_archived'),0)=1"
	} else if q.Filter != "deleted" {
		where += " AND coalesce(json_extract(r.payload,'$.data.is_archived'),0)=0"
	}
	if q.Filter == "deleted" {
		where += " AND (json_extract(r.payload,'$.data.deleted_at') IS NOT NULL OR EXISTS(SELECT 1 FROM msg_by_chat m WHERE m.chat_id=s.id AND json_extract(m.payload,'$.data.deleted_at') IS NOT NULL))"
	} else {
		where += " AND json_extract(r.payload,'$.data.deleted_at') IS NULL"
	}
	if q.Filter == "unread" {
		where += " AND (s.unread_count>0 OR json_extract(r.payload,'$.data.is_unread')=1)"
	}
	if q.Filter == "spam" {
		where += " AND (coalesce(json_extract(r.payload,'$.data.is_spam'),0)=1 OR EXISTS(SELECT 1 FROM msg_by_chat m WHERE m.chat_id=s.id AND json_extract(m.payload,'$.data.is_spam')=1))"
	}
	if q.Filter == "known" || q.Filter == "unknown" {
		known := "(coalesce(json_extract(r.payload,'$.data.filter'),'known')='known' OR EXISTS(SELECT 1 FROM json_each(r.payload,'$.data.participants') p CROSS JOIN records h ON h.collection='messages.handle' AND h.id=p.value AND h.deleted=0 WHERE json_extract(h.payload,'$.data.contact_id') IS NOT NULL))"
		if q.Filter == "unknown" {
			where += " AND NOT " + known
		} else {
			where += " AND " + known
		}
	}
	if q.After != "" {
		where += " AND (s.last_activity_at<? OR (s.last_activity_at=? AND s.id>?))"
		args = append(args, c.At, c.At, c.ID)
	}
	args = append(args, limit+1)
	rows, err := tx.QueryContext(ctx, "SELECT r.payload,s.last_activity_at,max(s.unread_count,coalesce(json_extract(r.payload,'$.data.is_unread'),0)),EXISTS(SELECT 1 FROM msg_by_chat m WHERE m.chat_id=s.id AND json_extract(m.payload,'$.data.has_unseen_mention')=1 AND json_extract(m.payload,'$.data.deleted_at') IS NULL) FROM chat_summary s INDEXED BY chat_activity CROSS JOIN records r ON r.collection='messages.chat' AND r.id=s.id WHERE "+where+" ORDER BY s.last_activity_at DESC,s.id LIMIT ?", args...)
	if err != nil {
		return out, err
	}
	for rows.Next() {
		var raw []byte
		var summary contracts.ChatSummary
		if err = rows.Scan(&raw, &summary.LastActivityAt, &summary.UnreadCount, &summary.HasUnseenMention); err != nil {
			break
		}
		if err = json.Unmarshal(raw, &summary.Chat); err != nil {
			break
		}
		out.Chats = append(out.Chats, summary)
	}
	if err == nil {
		err = rows.Err()
	}
	rows.Close()
	if err != nil {
		return out, err
	}
	if len(out.Chats) > limit {
		out.Chats = out.Chats[:limit]
		last := out.Chats[len(out.Chats)-1]
		out.Next = encodeView("chats", scope, last.LastActivityAt, last.Chat.ID)
	}
	people, err := summaryPeople(ctx, tx, v, out.Chats)
	if err != nil {
		return out, err
	}
	activities, err := summaryActivities(ctx, tx, out.Chats)
	if err != nil {
		return out, err
	}
	pins, err := summaryPinPreviews(ctx, tx, out.Chats)
	if err != nil {
		return out, err
	}
	for i := range out.Chats {
		out.Chats[i].PinPreview = pins[out.Chats[i].Chat.ID]
		if err = fillSummary(ctx, tx, v, &out.Chats[i], people, activities[out.Chats[i].Chat.ID]); err != nil {
			return out, err
		}
	}
	return out, nil
}
func fillSummary(ctx context.Context, tx *sql.Tx, v trust.Visibility, s *contracts.ChatSummary, people map[string]contracts.Record, activity summaryActivity) error {
	d := modelFields(s.Chat)
	s.KnownSender = d["filter"] == nil || fieldString(d, "filter") == "known"
	if n, ok := d["pinned_index"].(float64); ok {
		value := int(n)
		s.PinnedIndex = &value
	}
	if until, err := time.Parse(time.RFC3339Nano, fieldString(d, "muted_until")); err == nil {
		value := until.After(time.Now())
		s.Muted = &value
	}
	s.Title = fieldString(d, "display_name")
	s.AvatarHandles = []contracts.AvatarHandle{}
	names := []string{}
	if participants, ok := d["participants"].([]any); ok {
		for _, p := range participants {
			id, _ := p.(string)
			h, exists := people["messages.handle/"+id]
			if !exists {
				continue
			}
			hd := modelFields(h)
			s.KnownSender = s.KnownSender || hd["contact_id"] != nil
			name := fieldString(hd, "address")
			var contactID *string
			if v[contracts.Contacts] {
				if cid := fieldString(hd, "contact_id"); cid != "" {
					contact, exists := people["contacts.contact/"+cid]
					if exists {
						contactID = &cid
						cd := modelFields(contact)
						parts := []string{}
						for _, k := range []string{"given_name", "middle_name", "family_name"} {
							if value := fieldString(cd, k); value != "" {
								parts = append(parts, value)
							}
						}
						if len(parts) > 0 {
							name = strings.Join(parts, " ")
						} else if n := fieldString(cd, "nickname"); n != "" {
							name = n
						} else if n := fieldString(cd, "organization"); n != "" {
							name = n
						}
					}
				}
			}
			if len(s.AvatarHandles) < 3 {
				s.AvatarHandles = append(s.AvatarHandles, contracts.AvatarHandle{ID: id, ContactID: contactID})
			}
			if name != "" {
				names = append(names, name)
			}
		}
	}
	if s.Title == "" {
		switch len(names) {
		case 0:
		case 1:
			s.Title = names[0]
		case 2:
			s.Title = names[0] + " & " + names[1]
		case 3:
			s.Title = names[0] + ", " + names[1] + " & " + names[2]
		default:
			s.Title = fmt.Sprintf("%s, %s & %d others", names[0], names[1], len(names)-2)
		}
	}
	message := activity.message
	if message.ID == "" {
		return nil
	}
	messageID, activityID := message.ID, activity.activityID
	md := modelFields(message)
	last := &contracts.LastMessage{ID: message.ID, SentAt: fieldString(md, "sent_at"), IsFromMe: fieldBool(md, "is_from_me"), Preview: contracts.MessagePreview{Kind: "text", Text: fieldString(md, "text")}}
	if sender := fieldString(md, "sender_handle_id"); sender != "" {
		last.SenderHandleID = &sender
	}
	switch {
	case md["retracted_at"] != nil || hasItems(md["retracted_parts"]):
		last.Preview.Kind = "unsent"
	case fieldString(md, "kind") == "group_event":
		last.Preview.Kind = "group_event"
		last.Preview.GroupEvent, _ = md["group_event"].(map[string]any)
	case md["edited_at"] != nil:
		last.Preview.Kind = "edited"
	default:
		attachments := activity.attachments
		if len(attachments) > 0 {
			last.Preview.Kind = "attachments"
			last.Preview.Count = len(attachments)
			last.Preview.AttachmentKinds = []string{}
			for _, a := range attachments {
				k := attachmentKind(modelFields(a))
				if !slices.Contains(last.Preview.AttachmentKinds, k) {
					last.Preview.AttachmentKinds = append(last.Preview.AttachmentKinds, k)
				}
			}
		}
	}
	if activityID != messageID {
		r, err := modelRecord(ctx, tx, "messages.reaction", activityID)
		if err != nil {
			return err
		}
		rd := modelFields(r)
		verb := map[string]string{"love": "Loved", "like": "Liked", "dislike": "Disliked", "laugh": "Laughed at", "emphasize": "Emphasized", "question": "Questioned", "emoji": "Reacted " + fieldString(rd, "emoji") + " to", "sticker": "Reacted with a sticker to"}[fieldString(rd, "kind")]
		if fieldBool(rd, "removed") {
			verb = "Removed reaction from"
		}
		last.Preview = contracts.MessagePreview{Kind: "reaction", Reaction: &contracts.ReactionPreview{Verb: verb, Kind: fieldString(rd, "kind"), Removed: fieldBool(rd, "removed"), TargetExcerpt: fieldString(md, "text")}}
	}
	s.LastMessage = last
	return nil
}
func hasItems(value any) bool { a, _ := value.([]any); return len(a) > 0 }
func (s *Store) Thread(ctx context.Context, v trust.Visibility, q contracts.ThreadQuery) (contracts.ThreadResult, error) {
	out := contracts.ThreadResult{Items: []contracts.ThreadItem{}}
	limit, err := viewLimit(q.Limit)
	if err != nil || q.ChatID == "" || q.Before != "" && q.After != "" || q.Anchor != "" && (q.Before != "" || q.After != "") {
		return out, contracts.ErrInvalid
	}
	scope := threadCursorScope(q.ChatID, q.ThreadOriginatorID)
	selection, selectionArgs := threadSelection(q.ChatID, q.ThreadOriginatorID)
	var edge viewCursor
	token := q.Before
	if token == "" {
		token = q.After
	}
	if token != "" {
		edge, err = decodeView(token, "thread", scope)
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
	if _, err = modelRecord(ctx, tx, "messages.chat", q.ChatID); err != nil {
		return out, err
	}
	if q.ThreadOriginatorID != "" {
		var exists bool
		if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM msg_by_chat WHERE chat_id=? AND id=?)", q.ChatID, q.ThreadOriginatorID).Scan(&exists); err != nil {
			return out, err
		}
		if !exists {
			return out, contracts.ErrNotFound
		}
	}
	argsFor := func(args ...any) []any { return append(append([]any{}, selectionArgs...), args...) }
	query := func(condition, order string, n int, args ...any) ([]contracts.Record, error) {
		a := argsFor(args...)
		a = append(a, n)
		return readModelRecords(ctx, tx, "SELECT payload FROM "+selection+" "+condition+" ORDER BY sent_at "+order+",id "+order+" LIMIT ?", a...)
	}
	var messages []contracts.Record
	switch {
	case q.Before != "":
		messages, err = query("AND (sent_at,id)<(?,?)", "DESC", limit, edge.At, edge.ID)
		slices.Reverse(messages)
	case q.After != "":
		messages, err = query("AND (sent_at,id)>(?,?)", "ASC", limit, edge.At, edge.ID)
	case q.Anchor == "oldest":
		messages, err = query("", "ASC", limit)
	case q.Anchor != "" && q.Anchor != "latest":
		var at, id string
		if q.Anchor == "unread" {
			err = tx.QueryRowContext(ctx, "SELECT sent_at,id FROM "+selection+" AND unread=1 ORDER BY sent_at,id LIMIT 1", selectionArgs...).Scan(&at, &id)
			if err == nil {
				out.UnreadAnchor = id
			}
		} else {
			err = tx.QueryRowContext(ctx, "SELECT sent_at,id FROM "+selection+" AND id=?", argsFor(q.Anchor)...).Scan(&at, &id)
		}
		if err == sql.ErrNoRows && q.Anchor == "unread" {
			messages, err = query("", "DESC", limit)
			slices.Reverse(messages)
		} else if err == sql.ErrNoRows {
			return out, contracts.ErrNotFound
		} else if err == nil {
			older, e := query("AND (sent_at,id)<(?,?)", "DESC", limit/2, at, id)
			if e != nil {
				return out, e
			}
			slices.Reverse(older)
			newer, e := query("AND (sent_at,id)>=(?,?)", "ASC", limit-len(older), at, id)
			if e != nil {
				return out, e
			}
			messages = append(older, newer...)
		}
	default:
		messages, err = query("", "DESC", limit)
		slices.Reverse(messages)
	}
	if err != nil {
		return out, err
	}

	if len(messages) > 0 {
		args := make([]any, len(messages))
		for i, r := range messages {
			args[i] = r.ID
		}
		placeholders := strings.TrimSuffix(strings.Repeat("?,", len(messages)), ",")
		reactions, err := readModelRecords(ctx, tx, "SELECT payload FROM reaction_by_message WHERE message_id IN ("+placeholders+") ORDER BY at,seq,id", args...)
		if err != nil {
			return out, err
		}
		attachments, err := readModelRecords(ctx, tx, "SELECT payload FROM attachment_by_message WHERE message_id IN ("+placeholders+") ORDER BY id", args...)
		if err != nil {
			return out, err
		}
		byReaction := map[string][]contracts.Record{}
		for _, r := range reactions {
			id := fieldString(modelFields(r), "message_id")
			byReaction[id] = append(byReaction[id], r)
		}
		byAttachment := map[string][]contracts.Record{}
		for _, r := range attachments {
			id := attachmentMessage(modelFields(r))
			byAttachment[id] = append(byAttachment[id], r)
		}
		counts := map[string]int{}
		rows, err := tx.QueryContext(ctx, "SELECT originator,count(*) FROM msg_by_chat WHERE originator IN ("+placeholders+") AND originator<>id GROUP BY originator", args...)
		if err != nil {
			return out, err
		}
		for rows.Next() {
			var id string
			var n int
			if err = rows.Scan(&id, &n); err != nil {
				break
			}
			counts[id] = n
		}
		if err == nil {
			err = rows.Err()
		}
		rows.Close()
		if err != nil {
			return out, err
		}
		for _, r := range messages {
			item := contracts.ThreadItem{Record: r, Reactions: foldReactions(byReaction[r.ID]), Attachments: []contracts.Record{}, ReplyCount: counts[r.ID]}
			if a := byAttachment[r.ID]; a != nil {
				item.Attachments = a
			}
			out.Items = append(out.Items, item)
		}
	}

	if len(messages) > 0 {
		first, last := messages[0], messages[len(messages)-1]
		var exists bool
		at := modelTime(fieldString(modelFields(first), "sent_at"))
		if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM "+selection+" AND (sent_at,id)<(?,?))", argsFor(at, first.ID)...).Scan(&exists); err != nil {
			return out, err
		}
		if exists {
			out.Before = encodeView("thread", scope, at, first.ID)
		}
		at = modelTime(fieldString(modelFields(last), "sent_at"))
		if err = tx.QueryRowContext(ctx, "SELECT EXISTS(SELECT 1 FROM "+selection+" AND (sent_at,id)>(?,?))", argsFor(at, last.ID)...).Scan(&exists); err != nil {
			return out, err
		}
		if exists {
			out.After = encodeView("thread", scope, at, last.ID)
		}
	}
	return out, nil
}
func foldReactions(records []contracts.Record) []contracts.Record {
	current := map[string]contracts.Record{}
	for _, r := range records {
		d := modelFields(r)
		kind := fieldString(d, "kind")
		family := "classic"
		switch kind {
		case "emoji":
			family = "emoji:" + fieldString(d, "emoji")
		case "sticker":
			identity, _ := json.Marshal([]any{d["sticker_id"], d["attachment_id"], d["emoji"], d["sticker"], d["blob"]})
			family = "sticker:" + string(identity)
		}
		sender := fieldString(d, "sender_handle_id")
		if fieldBool(d, "is_from_me") {
			sender = "self"
		}
		key := fmt.Sprintf("%s/%v/%s", sender, d["part_index"], family)
		if fieldBool(d, "removed") {
			if previous, ok := current[key]; ok && fieldString(modelFields(previous), "kind") == kind {
				delete(current, key)
			}
		} else {
			current[key] = r
		}
	}
	out := []contracts.Record{}
	for _, r := range current {
		out = append(out, r)
	}
	slices.SortFunc(out, func(a, b contracts.Record) int { return strings.Compare(a.ID, b.ID) })
	return out
}
func (s *Store) Collection(ctx context.Context, v trust.Visibility, q contracts.CollectionQuery) (contracts.CollectionResult, error) {
	out := contracts.CollectionResult{Items: []contracts.Record{}, CoverageNotes: []string{}}
	limit, err := viewLimit(q.Limit)
	if err != nil || q.ChatID == "" || !slices.Contains([]string{"photos", "videos", "media", "audio", "documents", "links", "locations"}, q.Kind) {
		return out, contracts.ErrInvalid
	}
	var edge viewCursor
	if q.After != "" {
		edge, err = decodeView(q.After, "collection", q.ChatID+":"+q.Kind)
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
	if _, err = modelRecord(ctx, tx, "messages.chat", q.ChatID); err != nil {
		return out, err
	}
	if q.Kind == "links" || q.Kind == "locations" {
		where := "m.chat_id=? AND json_extract(m.payload,'$.data.deleted_at') IS NULL AND json_extract(m.payload,'$.data.balloon.kind')=?"
		kind := "link"
		if q.Kind == "locations" {
			kind = "find_my"
		}
		args := []any{q.ChatID, kind}
		if q.After != "" {
			where += " AND (m.sent_at<? OR (m.sent_at=? AND m.id>?))"
			args = append(args, edge.At, edge.At, edge.ID)
		}
		args = append(args, limit+1)
		records, err := readModelRecords(ctx, tx, "SELECT m.payload FROM msg_by_chat m WHERE "+where+" ORDER BY m.sent_at DESC,m.id LIMIT ?", args...)
		if err != nil {
			return out, err
		}
		if len(records) > limit {
			records = records[:limit]
			last := records[len(records)-1]
			out.Next = encodeView("collection", q.ChatID+":"+q.Kind, modelTime(fieldString(modelFields(last), "sent_at")), last.ID)
		}
		out.Items = records
		return out, nil
	}
	where := ""
	selection, kind := "a.payload", "a.kind=?"
	args := []any{q.ChatID, q.Kind}
	if q.Kind == "media" {
		selection = "json_set(a.payload,'$.data.message_id',m.id,'$.data.sent_at',m.sent_at)"
		kind = "a.kind IN ('photos','videos')"
		args = []any{q.ChatID}
	}
	if q.After != "" {
		where = " AND (m.sent_at<? OR (m.sent_at=? AND a.id>?))"
		args = append(args, edge.At, edge.At, edge.ID)
	}
	args = append(args, limit+1)
	records, err := readModelRecords(ctx, tx, "SELECT "+selection+" FROM msg_by_chat m JOIN attachment_by_message a ON a.message_id=m.id WHERE m.chat_id=? AND "+kind+" AND coalesce(json_extract(a.payload,'$.data.hidden'),0)=0 AND json_extract(m.payload,'$.data.deleted_at') IS NULL"+where+" ORDER BY m.sent_at DESC,a.id LIMIT ?", args...)
	if err != nil {
		return out, err
	}
	if len(records) > limit {
		records = records[:limit]
		last := records[len(records)-1]
		var at string
		if err = tx.QueryRowContext(ctx, "SELECT sent_at FROM msg_by_chat WHERE id=?", fieldString(modelFields(last), "message_id")).Scan(&at); err != nil {
			return out, err
		}
		out.Next = encodeView("collection", q.ChatID+":"+q.Kind, at, last.ID)
	}
	out.Items = records
	return out, nil
}
