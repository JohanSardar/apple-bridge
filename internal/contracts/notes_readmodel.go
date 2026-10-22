package contracts

type NotesListQuery struct {
	Folder   string `json:"folder,omitempty"`
	Sort     string `json:"sort,omitempty" enum:"modified,created,title"`
	TimeZone string `json:"time_zone,omitempty"`
}

type NoteThumbnail struct {
	Digest  string `json:"digest"`
	Variant string `json:"variant"`
}

type NoteListItem struct {
	Note      Record         `json:"note"`
	Preview   string         `json:"preview"`
	Thumbnail *NoteThumbnail `json:"thumbnail"`
	Pinned    *bool          `json:"pinned"`
	ReadOnly  bool           `json:"read_only"`
}

type NoteListSection struct {
	Key   string         `json:"key"`
	Title string         `json:"title"`
	Notes []NoteListItem `json:"notes"`
}

type NoteListFolder struct {
	Folder          Record  `json:"folder"`
	Count           int     `json:"count"`
	AccountID       string  `json:"account_id"`
	ParentID        *string `json:"parent_id"`
	ReadOnly        bool    `json:"read_only"`
	RecentlyDeleted bool    `json:"recently_deleted"`
}

type NoteListAccount struct {
	ID        string   `json:"id"`
	Title     string   `json:"title"`
	Count     int      `json:"count"`
	FolderIDs []string `json:"folder_ids"`
}

type NotesListResult struct {
	Accounts []NoteListAccount `json:"accounts"`
	Folders  []NoteListFolder  `json:"folders"`
	Sections []NoteListSection `json:"sections"`
	Count    int               `json:"count"`
	Sort     string            `json:"sort"`
	TimeZone string            `json:"time_zone"`
	Cursor   Cursor            `json:"cursor"`
}
