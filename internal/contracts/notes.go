package contracts

import "strings"

type NoteArgs struct {
	ID        string  `json:"id,omitempty"`
	FolderID  string  `json:"folder_id,omitempty"`
	Title     *string `json:"title,omitempty"`
	PlainText *string `json:"plain_text,omitempty"`
	Mode      string  `json:"mode,omitempty"`
}

func ValidateNoteCommand(c Command) (NoteArgs, error) {
	var a NoteArgs
	if !ValidCommandID(c.ID) || c.WaitMS < 0 || c.WaitMS > 15000 || DecodeInput(c.Args, &a) != nil {
		return a, ErrInvalid
	}
	switch c.Op {
	case "notes.create":
		if a.ID != "" || a.FolderID == "" || a.Title == nil || strings.TrimSpace(*a.Title) == "" || a.Mode != "" || c.ExpectedVersion != 0 {
			return a, ErrInvalid
		}
	case "notes.create_folder":
		if a.ID != "" || a.FolderID != "" || a.Title == nil || strings.TrimSpace(*a.Title) == "" || a.PlainText != nil || a.Mode != "" || c.ExpectedVersion != 0 {
			return a, ErrInvalid
		}
	case "notes.update":
		if a.ID == "" || c.ExpectedVersion == 0 || a.FolderID != "" || a.Title != nil || a.PlainText == nil || (a.Mode != "" && a.Mode != "replace" && a.Mode != "append") {
			return a, ErrInvalid
		}
	case "notes.rename_folder":
		if a.ID == "" || c.ExpectedVersion == 0 || a.FolderID != "" || a.Title == nil || !strings.HasPrefix(*a.Title, "Bridge Test [ab-test") || a.PlainText != nil || a.Mode != "" {
			return a, ErrInvalid
		}
	case "notes.move":
		if a.ID == "" || c.ExpectedVersion == 0 || a.FolderID == "" || a.Title != nil || a.PlainText != nil || a.Mode != "" {
			return a, ErrInvalid
		}
	case "notes.delete":
		if a.ID == "" || c.ExpectedVersion == 0 || a.FolderID != "" || a.Title != nil || a.PlainText != nil || a.Mode != "" {
			return a, ErrInvalid
		}
	default:
		return a, ErrUnsupported
	}
	return a, nil
}
