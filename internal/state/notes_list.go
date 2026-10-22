package state

import (
	"cmp"
	"context"
	"database/sql"
	"encoding/json"
	"slices"
	"strings"
	"time"

	"github.com/JohanSardar/apple-bridge/internal/contracts"
	"github.com/JohanSardar/apple-bridge/internal/trust"
)

type noteListCandidate struct {
	item  contracts.NoteListItem
	title string
	date  time.Time
}

func (s *Store) NotesList(ctx context.Context, visibility trust.Visibility, query contracts.NotesListQuery) (contracts.NotesListResult, error) {
	out := contracts.NotesListResult{Accounts: []contracts.NoteListAccount{}, Folders: []contracts.NoteListFolder{}, Sections: []contracts.NoteListSection{}, Sort: query.Sort, TimeZone: query.TimeZone}
	if out.Sort == "" {
		out.Sort = "modified"
	}
	if out.TimeZone == "" {
		out.TimeZone = "UTC"
	}
	zone, err := time.LoadLocation(out.TimeZone)
	if err != nil || !slices.Contains([]string{"modified", "created", "title"}, out.Sort) {
		return out, contracts.ErrInvalid
	}
	tx, err := s.db.BeginTx(ctx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		return out, err
	}
	defer tx.Rollback()
	out.Cursor, err = cursorAt(ctx, tx)
	if err != nil || !visibility[contracts.Notes] {
		return out, err
	}
	records, err := readModelRecords(ctx, tx, "SELECT payload FROM records WHERE collection IN ('notes.folder','notes.note') AND deleted=0 ORDER BY collection,id")
	if err != nil {
		return out, err
	}
	folders := map[string]contracts.NoteListFolder{}
	for _, record := range records {
		if record.Collection != "notes.folder" {
			continue
		}
		fields := modelFields(record)
		folder := contracts.NoteListFolder{Folder: record, AccountID: fieldString(fields, "account"), RecentlyDeleted: noteFlag(fields, "recently_deleted")}
		folder.ReadOnly = folder.RecentlyDeleted || noteFlag(fields, "read_only")
		if parent := fieldString(fields, "parent_id"); parent != "" && parent != record.ID {
			folder.ParentID = &parent
		}
		folders[record.ID] = folder
	}
	if query.Folder != "" {
		if _, exists := folders[query.Folder]; !exists {
			return out, contracts.ErrNotFound
		}
	}
	accounts := map[string]contracts.NoteListAccount{}
	for id, folder := range folders {
		seen := map[string]bool{id: true}
		for parent := folder.ParentID; parent != nil; {
			if seen[*parent] {
				return out, contracts.ErrCorrupt
			}
			seen[*parent] = true
			ancestor, exists := folders[*parent]
			if !exists || ancestor.AccountID != folder.AccountID {
				folder.ParentID = nil
				break
			}
			folder.ReadOnly = folder.ReadOnly || ancestor.ReadOnly
			folder.RecentlyDeleted = folder.RecentlyDeleted || ancestor.RecentlyDeleted
			parent = ancestor.ParentID
		}
		folders[id] = folder
		account := accounts[folder.AccountID]
		account.ID = folder.AccountID
		if title := fieldString(modelFields(folder.Folder), "account_name"); title != "" {
			account.Title = title
		}
		if account.Title == "" {
			account.Title = "Notes"
		}
		account.FolderIDs = append(account.FolderIDs, id)
		accounts[folder.AccountID] = account
	}
	items := []noteListCandidate{}
	for _, record := range records {
		if record.Collection != "notes.note" {
			continue
		}
		fields := modelFields(record)
		folderID := fieldString(fields, "folder_id")
		folder, found := folders[folderID]
		recent := folder.RecentlyDeleted || noteFlag(fields, "recently_deleted")
		folder.Count++
		if found {
			folders[folderID] = folder
			if !recent {
				account := accounts[folder.AccountID]
				account.Count++
				accounts[folder.AccountID] = account
			}
		}
		if (query.Folder == "" && recent) || (query.Folder != "" && query.Folder != folderID) {
			continue
		}
		item := contracts.NoteListItem{Note: record, ReadOnly: recent || folder.ReadOnly || fieldString(fields, "editable") != "plain"}
		if pinned, known := fields["pinned"].(bool); known {
			item.Pinned = &pinned
		}
		if noteFlag(fields, "locked") || fieldString(fields, "content_status") == "locked" {
			item.ReadOnly = true
			item.Note.Data, _ = json.Marshal(map[string]any{"title": fieldString(fields, "title"), "folder_id": folderID, "locked": true, "content_status": "locked", "editable": "none", "created_at": fieldString(fields, "created_at"), "modified_at": fieldString(fields, "modified_at")})
		} else {
			item.Preview = notePreview(fieldString(fields, "plain_text"), fieldString(fields, "title"))
			item.Thumbnail = noteThumbnail(fields)
		}
		field := "modified_at"
		if out.Sort == "created" {
			field = "created_at"
		}
		items = append(items, noteListCandidate{item: item, title: strings.ToLower(fieldString(fields, "title")), date: noteDate(fields, field)})
	}
	slices.SortFunc(items, func(a, b noteListCandidate) int {
		ap, bp := a.item.Pinned != nil && *a.item.Pinned, b.item.Pinned != nil && *b.item.Pinned
		if ap != bp {
			if ap {
				return -1
			}
			return 1
		}
		order := 0
		if out.Sort == "title" {
			order = strings.Compare(a.title, b.title)
		} else {
			order = b.date.Compare(a.date)
		}
		return cmp.Or(order, strings.Compare(a.item.Note.ID, b.item.Note.ID))
	})
	now := time.Now().In(zone)
	for _, item := range items {
		key, title := noteDateSection(item, out.Sort, now, zone)
		if len(out.Sections) == 0 || out.Sections[len(out.Sections)-1].Key != key {
			out.Sections = append(out.Sections, contracts.NoteListSection{Key: key, Title: title, Notes: []contracts.NoteListItem{}})
		}
		i := len(out.Sections) - 1
		out.Sections[i].Notes = append(out.Sections[i].Notes, item.item)
	}
	out.Count = len(items)
	for _, folder := range folders {
		out.Folders = append(out.Folders, folder)
	}
	slices.SortFunc(out.Folders, func(a, b contracts.NoteListFolder) int {
		return cmp.Or(strings.Compare(a.AccountID, b.AccountID), strings.Compare(strings.ToLower(fieldString(modelFields(a.Folder), "title")), strings.ToLower(fieldString(modelFields(b.Folder), "title"))), strings.Compare(a.Folder.ID, b.Folder.ID))
	})
	for _, account := range accounts {
		slices.Sort(account.FolderIDs)
		out.Accounts = append(out.Accounts, account)
	}
	slices.SortFunc(out.Accounts, func(a, b contracts.NoteListAccount) int {
		return cmp.Or(strings.Compare(a.Title, b.Title), strings.Compare(a.ID, b.ID))
	})
	return out, nil
}

func noteFlag(fields map[string]any, key string) bool { value, _ := fields[key].(bool); return value }

func noteDate(fields map[string]any, key string) time.Time {
	value, _ := time.Parse(time.RFC3339Nano, fieldString(fields, key))
	return value
}

func notePreview(text, title string) string {
	lines := strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n")
	if len(lines) > 0 && strings.TrimSpace(lines[0]) == strings.TrimSpace(title) {
		lines = lines[1:]
	}
	for _, line := range lines {
		value := []rune(strings.Join(strings.Fields(strings.ReplaceAll(line, "\ufffc", "")), " "))
		if len(value) > 0 {
			if len(value) > 160 {
				value = value[:160]
			}
			return string(value)
		}
	}
	return ""
}

func noteThumbnail(fields map[string]any) *contracts.NoteThumbnail {
	if thumbnail, ok := fields["thumbnail"].(map[string]any); ok && contracts.ValidDigest(fieldString(thumbnail, "digest")) && fieldString(thumbnail, "state") != "unavailable" {
		return &contracts.NoteThumbnail{Digest: fieldString(thumbnail, "digest"), Variant: "thumb"}
	}
	attachments, _ := fields["attachments"].([]any)
	for _, value := range attachments {
		attachment, _ := value.(map[string]any)
		derivatives, _ := attachment["derivatives"].(map[string]any)
		if thumbnail, ok := derivatives["thumb"].(map[string]any); ok && contracts.ValidDigest(fieldString(thumbnail, "digest")) && fieldString(thumbnail, "state") != "unavailable" {
			return &contracts.NoteThumbnail{Digest: fieldString(thumbnail, "digest"), Variant: "thumb"}
		}
	}
	return nil
}

func noteDateSection(item noteListCandidate, sort string, now time.Time, zone *time.Location) (string, string) {
	if item.item.Pinned != nil && *item.item.Pinned {
		return "pinned", "Pinned"
	}
	if sort == "title" {
		return "notes", "Notes"
	}
	date := item.date.In(zone)
	today := time.Date(now.Year(), now.Month(), now.Day(), 0, 0, 0, 0, zone)
	for _, period := range []struct {
		days       int
		key, title string
	}{{0, "today", "Today"}, {1, "yesterday", "Yesterday"}, {7, "previous_7_days", "Previous 7 Days"}, {30, "previous_30_days", "Previous 30 Days"}} {
		if !date.Before(today.AddDate(0, 0, -period.days)) {
			return period.key, period.title
		}
	}
	return "older", "Earlier"
}
