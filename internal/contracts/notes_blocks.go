package contracts

type NotesRun struct {
	Text          string   `json:"text"`
	Bold          bool     `json:"bold"`
	Italic        bool     `json:"italic"`
	Underline     bool     `json:"underline"`
	Strikethrough bool     `json:"strikethrough"`
	Font          *string  `json:"font,omitempty"`
	FontSize      *float64 `json:"font_size,omitempty"`
	Color         *string  `json:"color,omitempty"`
	Superscript   *int     `json:"superscript,omitempty"`
	Link          *string  `json:"link,omitempty"`
	Highlight     *string  `json:"highlight,omitempty"`
	AttachmentID  *string  `json:"attachment_id,omitempty"`
	AttachmentUTI *string  `json:"attachment_uti,omitempty"`
	Unsupported   *string  `json:"unsupported,omitempty"`
}

type NotesCell struct {
	Blocks []NotesBlock `json:"blocks"`
}

type NotesBlock struct {
	Type         string        `json:"type"`
	Text         string        `json:"text"`
	Runs         []NotesRun    `json:"runs"`
	Level        *int          `json:"level,omitempty"`
	Alignment    *string       `json:"alignment,omitempty"`
	Blockquote   *bool         `json:"blockquote,omitempty"`
	ListStyle    *string       `json:"list_style,omitempty"`
	Checked      *bool         `json:"checked,omitempty"`
	ChecklistID  *string       `json:"checklist_id,omitempty"`
	AttachmentID *string       `json:"attachment_id,omitempty"`
	Rows         [][]NotesCell `json:"rows,omitempty"`
	Children     []NotesBlock  `json:"children"`
	Collapsible  *bool         `json:"collapsible,omitempty"`
	Unsupported  *string       `json:"unsupported,omitempty"`
}
